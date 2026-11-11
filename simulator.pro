QT += core gui
CONFIG += c++17 release
CONFIG -= app_bundle
TARGET = touchmeter-simulator
TEMPLATE = app
DEFINES += TOUCHMETER_SIMULATOR_ONLY
SOURCES += src/main.cpp src/backend.cpp src/appearance.cpp src/controls.cpp \
           src/runeui.cpp src/testdata.cpp src/burnrate.cpp src/appconfig.cpp \
           src/volume.cpp src/simulator.cpp
HEADERS += src/backend.h src/appearance.h src/controls.h src/runeui.h \
           src/testdata.h src/burnrate.h src/appconfig.h src/volume.h src/simulator.h
LIBS += $$PWD/ui/target/release/libtouchmeter_ui.a
unix:!macx: LIBS += -ldl -lpthread -lm -lxkbcommon -lwayland-client
macx: LIBS += -framework AppKit -framework CoreFoundation -framework CoreGraphics \
              -framework CoreText -framework Foundation -framework Metal -framework QuartzCore \
              -framework Security -framework SystemConfiguration -framework IOKit -framework CoreVideo -liconv -lobjc
PRE_TARGETDEPS += $$PWD/ui/target/release/libtouchmeter_ui.a
RESOURCES += src/simulator-resources.qrc
