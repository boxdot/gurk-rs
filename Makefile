all: build run

build:
	cargo fmt
	cargo build

run:
	cargo run