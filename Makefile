.PHONY: simulator run-simulator test-simulator build test

# No Omarchy, Touch Bar, DRM, evdev, or generated Wayland protocols required.
simulator:
	./bin/build-simulator

run-simulator: simulator
	./build/simulator/touchmeter-simulator $(ARGS)

test-simulator: simulator
	TOUCHMETER_BINARY="$(CURDIR)/build/simulator/touchmeter-simulator" python3 tests/test_standalone.py

build:
	./bin/build

test:
	./bin/test
