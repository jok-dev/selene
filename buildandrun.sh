#!/bin/bash

CURRENT_DIR=$(pwd)
BUILD_SUCCESS=false

cargo build --release

pushd C:/Users/Jok/work/voldex/Driving-Empire/
$CURRENT_DIR/target/release/selene.exe generate-roblox-std
popd

cargo build --release && target/release/selene.exe C:/Users/Jok/work/voldex/Driving-Empire/src/ --config C:/Users/Jok/work/voldex/Driving-Empire/tools/selene/selene-errors-only.toml