#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
# Test images live in TMPDIR. Storage must be a different local filesystem.
: "${ODR_TEST_STORAGE:=/dev/shm}"
export ODR_TEST_STORAGE
test -d "$ODR_TEST_STORAGE"
cargo test --locked disposable_ -- --ignored --nocapture
