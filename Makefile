.PHONY: simulator run-simulator test-simulator build test

simulator:
	./bin/build-simulator

run-simulator: simulator
	./bin/simulate $(ARGS)

test-simulator: test

build:
	./bin/build

test:
	./bin/test
