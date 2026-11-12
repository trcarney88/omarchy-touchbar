.PHONY: simulator run-simulator test-simulator build test rust-simulator run-rust-simulator legacy-build legacy-test legacy-simulator legacy-run-simulator

# Rust is the canonical developer build. Hardware activation remains unvalidated.
rust-simulator:
	$(MAKE) build

run-rust-simulator:
	$(MAKE) run-simulator ARGS="$(ARGS)"

simulator:
	./bin/build-simulator

run-simulator: simulator
	./bin/simulate $(ARGS)

test-simulator: test

build:
	./bin/build

test:
	./bin/test

legacy-build:
	./bin/build-legacy

legacy-test:
	./bin/test-legacy

legacy-simulator:
	./bin/build-simulator-legacy

legacy-run-simulator: legacy-simulator
	./bin/simulate-legacy $(ARGS)
