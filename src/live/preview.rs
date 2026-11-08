//! A single worker with one replaceable request and one bounded result slot.
use crate::engine::{
    self,
    preview::{Cache, Content},
    process::Control,
};
use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread,
};

struct Request {
    generation: u64,
    directory: PathBuf,
    id: i64,
    image: bool,
    control: Control,
}
#[derive(Default)]
struct State {
    generation: u64,
    pending: Option<Request>,
    active: Option<Control>,
    result: Option<(u64, engine::Result<Content>)>,
    stopped: bool,
}
pub(super) struct Worker {
    state: Arc<(Mutex<State>, Condvar)>,
}
pub(super) struct Idle(Arc<(Mutex<State>, Condvar)>);
impl Idle {
    /// Called on the catalogue worker, never on GTK's main thread.
    pub fn wait(self) -> engine::Result<()> {
        let (lock, ready) = &*self.0;
        let state = lock.lock().unwrap();
        let (state, _) = ready
            .wait_timeout_while(state, std::time::Duration::from_secs(3), |state| {
                state.active.is_some()
            })
            .unwrap();
        if state.active.is_some() {
            return engine::fail(
                "The preview is still stopping. Try deleting the saved scan again shortly.",
            );
        }
        Ok(())
    }
}
impl Worker {
    pub fn new() -> Self {
        let mut cache = Cache::default();
        Self::with_loader(move |request| {
            engine::preview::load(
                &request.directory,
                request.id,
                request.image,
                &request.control,
                &mut cache,
            )
        })
    }
    fn with_loader(
        mut load: impl FnMut(&Request) -> engine::Result<Content> + Send + 'static,
    ) -> Self {
        let state = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let shared = state.clone();
        thread::spawn(move || {
            let (lock, ready) = &*shared;
            loop {
                let request = {
                    let mut state = lock.lock().unwrap();
                    while state.pending.is_none() && !state.stopped {
                        state = ready.wait(state).unwrap();
                    }
                    if state.stopped {
                        break;
                    }
                    let request = state.pending.take().unwrap();
                    state.active = Some(request.control.clone());
                    request
                };
                let result = load(&request);
                let mut state = lock.lock().unwrap();
                state.active = None;
                ready.notify_all();
                if !state.stopped
                    && state.generation == request.generation
                    && request.control.checkpoint().is_ok()
                {
                    state.result = Some((request.generation, result));
                }
            }
        });
        Self { state }
    }
    pub fn request(&self, generation: u64, directory: PathBuf, id: i64, image: bool) {
        let (lock, ready) = &*self.state;
        let mut state = lock.lock().unwrap();
        if let Some(control) = &state.active {
            control.cancel();
        }
        state.generation = generation;
        state.result = None;
        state.pending = Some(Request {
            generation,
            directory,
            id,
            image,
            control: Control::default(),
        });
        ready.notify_one();
    }
    pub fn cancel(&self) -> Idle {
        let mut state = self.state.0.lock().unwrap();
        if let Some(control) = &state.active {
            control.cancel();
        }
        state.pending = None;
        state.result = None;
        Idle(self.state.clone())
    }
    pub fn take(&self) -> Option<(u64, engine::Result<Content>)> {
        self.state.0.lock().unwrap().result.take()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
        self.state.0.lock().unwrap().stopped = true;
        self.state.1.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };

    #[test]
    fn rapid_selection_cancels_active_work_and_keeps_only_latest_request() {
        let (started, starts) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let worker = Worker::with_loader(move |request| {
            started.send(request.id).unwrap();
            if request.id == 1 {
                released.recv().unwrap();
                request.control.checkpoint()?;
                panic!("Obsolete preview was not cancelled");
            }
            Ok(Content::Text(request.id.to_string()))
        });
        worker.request(1, PathBuf::new(), 1, false);
        assert_eq!(starts.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
        for id in 2..=1000 {
            worker.request(id as u64, PathBuf::new(), id, false);
        }
        release.send(()).unwrap();
        assert_eq!(starts.recv_timeout(Duration::from_secs(2)).unwrap(), 1000);
        let start = Instant::now();
        loop {
            if let Some((generation, result)) = worker.take() {
                assert_eq!(generation, 1000);
                assert!(matches!(result.unwrap(), Content::Text(text) if text == "1000"));
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(2));
            thread::sleep(Duration::from_millis(1));
        }
        assert!(starts.try_recv().is_err());
    }

    #[test]
    fn closing_preview_cancels_work_and_publishes_no_result() {
        let (started, starts) = mpsc::channel();
        let (done, finished) = mpsc::channel();
        let worker = Worker::with_loader(move |request| {
            started.send(()).unwrap();
            while request.control.checkpoint().is_ok() {
                thread::sleep(Duration::from_millis(1));
            }
            done.send(()).unwrap();
            Err(engine::Error::Cancelled)
        });
        worker.request(1, PathBuf::new(), 1, false);
        starts.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(worker);
        finished.recv_timeout(Duration::from_secs(2)).unwrap();
    }
}
