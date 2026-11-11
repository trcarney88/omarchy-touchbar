#include "backend.h"
#ifndef TOUCHMETER_SIMULATOR_ONLY
#include "lease.h"
#include "touchinput.h"
#include "session.h"
#endif
#include "appearance.h"
#include "controls.h"
#include "runeui.h"
#include "testdata.h"
#include "appconfig.h"
#include "volume.h"
#include "simulator.h"
#include <QSocketNotifier>
#ifdef __linux__
#include <sys/signalfd.h>
#else
#include <fcntl.h>
#endif
#include <signal.h>
#include <unistd.h>
#include <QGuiApplication>
#include <QTimer>
#include <QDebug>
#include <QLockFile>
#include <QDir>
#include <QFileInfo>
#include <QFile>
#include <QScreen>
#include <QTextStream>
#include <QDateTime>
#include <cstdio>
#include <cerrno>

#ifndef __linux__
static int shutdownPipe = -1;
static void requestShutdown(int) {
    const int saved = errno;
    const char byte = 1;
    if (shutdownPipe >= 0) { const auto result = write(shutdownPipe, &byte, 1); (void)result; }
    errno = saved;
}
#endif

int main(int argc, char **argv) {
#ifdef TOUCHMETER_SIMULATOR_ONLY
    if (qEnvironmentVariableIsEmpty("XDG_RUNTIME_DIR")) {
        const auto runtime = QDir::tempPath() + "/touchmeter-simulator-" + QString::number(getuid());
        if (!QFileInfo::exists(runtime)) {
            if (!QDir().mkpath(runtime) || !QFile::setPermissions(runtime, QFileDevice::ReadOwner | QFileDevice::WriteOwner | QFileDevice::ExeOwner)) {
                fprintf(stderr, "Cannot create simulator runtime directory.\n"); return 1;
            }
        }
        if (QFileInfo(runtime).isSymLink() || !QFileInfo(runtime).isDir() || QFileInfo(runtime).ownerId() != uint(getuid())
            || (QFileInfo(runtime).permissions() & (QFileDevice::WriteGroup | QFileDevice::WriteOther))) {
            fprintf(stderr, "Unsafe simulator runtime directory. Set XDG_RUNTIME_DIR to a private directory.\n"); return 1;
        }
        qputenv("XDG_RUNTIME_DIR", runtime.toUtf8());
    }
    // Keep one entry point and scene; the standalone binary cannot open hardware.
    QList<QByteArray> storage;
    for (int i = 0; i < argc; ++i) storage.append(QByteArray(argv[i]));
    const auto command = storage.value(1);
    if (command.isEmpty() || command.startsWith("--")) storage.insert(1, "simulate");
    if (command == "open") { fprintf(stderr, "This build supports only the simulator, previews, test data, and config.\n"); return 1; }
    if (command == "test" && !storage.contains("--simulator")) storage.append("--simulator");
    QList<char *> arguments;
    for (auto &argument : storage) arguments.append(argument.data());
    argc = arguments.size(); arguments.append(nullptr); argv = arguments.data();
#endif
#ifdef __linux__
    // Block before Qt creates threads so signalfd receives service-stop signals.
    sigset_t signalMask;
    sigemptyset(&signalMask); sigaddset(&signalMask, SIGTERM); sigaddset(&signalMask, SIGINT);
    sigprocmask(SIG_BLOCK, &signalMask, nullptr);
#endif
    // CLI operations do not initialize graphics, lease hardware, or read credentials.
    if (argc > 1 && (QString::fromLocal8Bit(argv[1]) == "test" || QString::fromLocal8Bit(argv[1]) == "config")) {
        QCoreApplication cli(argc, argv);
        return cli.arguments()[1] == "test" ? TestData::runCli(cli.arguments().mid(2)) : AppConfig::runCli(cli.arguments().mid(2));
    }
    const bool simulator = argc > 1 && QString::fromLocal8Bit(argv[1]) == "simulate";
    if (simulator && argc > 2 && QString::fromLocal8Bit(argv[2]) == "--help") {
        QTextStream(stdout) << Simulator::help(); return 0;
    }
    if (!simulator) {
        qputenv("QT_QPA_PLATFORM", "offscreen");
        qputenv("QT_QPA_PLATFORMTHEME", "");
    }
    qputenv("QT_FORCE_STDERR_LOGGING", "1");
    QGuiApplication app(argc, argv);
    app.setApplicationName("touchmeter"); app.setApplicationVersion("0.8.0"); app.setDesktopFileName("touchmeter");
    const auto args = app.arguments();
    const bool preview = args.value(1) == "preview";
    const bool detached = preview || simulator;
#ifdef TOUCHMETER_SIMULATOR_ONLY
    if (!detached) { fprintf(stderr, "Use simulate, preview, test, or config.\n"); return 1; }
#endif
    if (!detached && args.value(1) != "open") {
        fprintf(stderr, "Usage: touchmeter open | touchmeter simulate --help | touchmeter config --help | touchmeter test --help | touchmeter preview <output.png> [--demo | --fixture <file.json> | --scenario <name>] [--menu|--ai|--functions|--providers|--volume] [--language <code>]\n"); return 1;
    }
    if (simulator) {
        for (int i = 2; i < args.size(); ++i) {
            const auto option = args[i];
            if (QStringList{"--scenario", "--fixture", "--monitor", "--language"}.contains(option)) {
                if (i + 1 >= args.size() || args[i + 1].startsWith("--")) { qCritical() << "Missing value for" << option; return 1; }
                ++i;
            } else if (!QStringList{"--demo", "--menu", "--ai", "--functions", "--providers", "--volume", "--fullscreen", "--list-monitors"}.contains(option)) {
                qCritical() << "Unknown simulator option:" << option; return 1;
            }
        }
    }
    QScreen *targetScreen = app.primaryScreen();
    if (simulator && args.contains("--list-monitors")) {
        QTextStream out(stdout); int index = 0;
        for (auto *screen : app.screens()) out << index++ << "  " << screen->name() << "  " << screen->geometry().width() << 'x' << screen->geometry().height()
            << (screen == app.primaryScreen() ? "  (primary)" : "") << '\n';
        return 0;
    }
    if (simulator && args.contains("--monitor")) {
        const auto name = args.value(args.indexOf("--monitor") + 1);
        bool numeric; const int index = name.toInt(&numeric);
        targetScreen = nullptr;
        if (numeric) { if (index >= 0 && index < app.screens().size()) targetScreen = app.screens()[index]; }
        else for (auto *screen : app.screens()) if (screen->name() == name) { targetScreen = screen; break; }
        if (!targetScreen) { qCritical() << "Unknown monitor; use touchmeter simulate --list-monitors"; return 1; }
    }
    if (preview && args.value(2).isEmpty()) { fprintf(stderr, "Preview needs an output path\n"); return 1; }
    const int fixtureIndex = args.indexOf("--fixture");
    const int scenarioIndex = args.indexOf("--scenario");
    if (fixtureIndex >= 0 && (!detached || (!simulator && args.contains("--demo")) || scenarioIndex >= 0 || args.value(fixtureIndex + 1).isEmpty() || !QFileInfo::exists(args.value(fixtureIndex + 1)))) {
        qCritical() << "--fixture needs an existing JSON file in preview/simulate, without --scenario (preview also excludes --demo)"; return 1;
    }
    TestData testData(detached ? (fixtureIndex >= 0 ? args.value(fixtureIndex + 1) : QString()) : TestData::runtimePath(), !preview);
    if (detached && !testData.error().isEmpty()) { qCritical().noquote() << testData.error(); return 1; }
    QJsonObject initialData = testData.current();
    QJsonObject simulatorDefault;
    if (scenarioIndex >= 0 || (simulator && initialData.isEmpty())) {
        if (!detached) { qCritical() << "--scenario is only supported in preview/simulate"; return 1; }
        QString error;
        initialData = TestData::scenario(scenarioIndex >= 0 ? args.value(scenarioIndex + 1) : "menu", &error);
        if (!error.isEmpty()) { qCritical().noquote() << error; return 1; }
        if (simulator) initialData["now"] = QDateTime::currentDateTimeUtc().toString(Qt::ISODate);
    }
    if (simulator) {
        QString error; simulatorDefault = TestData::scenario("menu", &error);
        simulatorDefault["now"] = QDateTime::currentDateTimeUtc().toString(Qt::ISODate);
    }
    // Save the requested launch page too, so clearing an override restores it.
    if (simulator) {
        for (const auto &page : {"menu", "ai", "functions", "providers", "volume"})
            if (args.contains(QString("--") + page)) initialData["mode"] = QString(page) == "ai" ? "usage" : page;
    }
    QJsonObject baseData = initialData;
    TestData simulatorOverrides(simulator ? TestData::runtimePath(true) : QString(), simulator);
    if (simulator && !simulatorOverrides.current().isEmpty()) initialData = simulatorOverrides.current();
    QLockFile lock(qEnvironmentVariable("XDG_RUNTIME_DIR", QDir::tempPath()) + "/touchmeter.lock");
    if (!detached && !lock.tryLock(0)) { qInfo() << "Touchmeter is already running"; return 0; }
    const int languageIndex = args.indexOf("--language");
    int pageFlags = 0;
    for (const auto &flag : {"--menu", "--ai", "--functions", "--providers", "--volume"}) if (args.contains(flag)) ++pageFlags;
    if (pageFlags > 1 || (!detached && pageFlags)) { qCritical() << "Choose one preview/simulator page flag"; return 1; }
    Appearance appearance(
#ifdef TOUCHMETER_SIMULATOR_ONLY
        true,
#else
        detached && args.contains("--demo"),
#endif
        detached && languageIndex >= 0 ? args.value(languageIndex + 1) : QString());
    app.installTranslator(&appearance);
    const bool demoBackend =
#ifdef TOUCHMETER_SIMULATOR_ONLY
        true;
#else
        simulator || (preview && args.contains("--demo"));
#endif
    Backend backend({}, demoBackend, !detached, nullptr, initialData);
    AppConfig config({}, !preview);
    backend.setFireTriggerMinutes(config.fireTriggerMinutes());
    QObject::connect(&config, &AppConfig::changed, &backend, [&] { backend.setFireTriggerMinutes(config.fireTriggerMinutes()); });
    Controls controls;
    Volume volume(detached || backend.testing());
    if (backend.testing()) volume.setSimulation(true, initialData.value("volume").toObject());
    controls.setVolumeAvailable(volume.available());
    QObject::connect(&volume, &Volume::changed, &controls, [&] { controls.setVolumeAvailable(volume.available()); });
    QObject::connect(&controls, &Controls::volumeRequested, &volume, &Volume::setVolume);
    QObject::connect(&controls, &Controls::muteRequested, &volume, &Volume::toggleMute);
    controls.setProviders(backend.providers());
    QObject::connect(&backend, &Backend::changed, &controls, [&] { controls.setProviders(backend.providers()); });
    QObject::connect(&controls, &Controls::providerSelected, &backend, &Backend::selectProvider);
    auto applyTestMode = [&](const QJsonObject &data) {
        controls.cancel();
        const auto mode = data.value("mode").toString();
        controls.openApp(data.isEmpty() || mode == "menu" ? "menu" : mode == "functions" ? "functions" : mode == "volume" ? "volume" : "ai");
        if (mode == "providers" && !controls.providerPicker()) controls.toggleProviders();
    };
    applyTestMode(initialData);
    QJsonObject effectiveData = initialData;
    auto applyData = [&](const QJsonObject &data) {
        if (data == effectiveData) return;
        effectiveData = data;
        backend.setTestData(data); volume.setSimulation(detached || !data.isEmpty(), data.value("volume").toObject()); applyTestMode(data);
    };
    auto applySimulatorData = [&] {
        applyData(simulatorOverrides.current().isEmpty() ? baseData : simulatorOverrides.current());
    };
    QObject::connect(&testData, &TestData::changed, &backend, [&](const QJsonObject &data) {
        if (simulator) { baseData = data.isEmpty() ? simulatorDefault : data; applySimulatorData(); }
        else applyData(data);
    });
    QObject::connect(&simulatorOverrides, &TestData::changed, &backend, [&](const QJsonObject &) { if (simulator) applySimulatorData(); });
    if (preview && args.contains("--functions")) controls.openApp("functions");
    if (preview && args.contains("--volume")) controls.openApp("volume");
    if (preview && args.contains("--menu")) controls.back();
    if (preview && args.contains("--ai")) controls.openApp("ai");
    if (preview && args.contains("--providers")) { controls.openApp("ai"); if (!controls.providerPicker()) controls.toggleProviders(); }
    QObject::connect(&appearance, &Appearance::changed, &backend, &Backend::reload);
    RuneUi ui(backend, appearance, controls, &volume);
    if (!ui.ready()) { qCritical() << "Could not initialize Rune"; return 1; }
    if (preview) {
        QTimer::singleShot(300, &app, [&] {
            const auto image = ui.render();
            if (image.isNull() || !image.save(args.value(2))) { qCritical() << "Could not save dashboard preview"; app.exit(1); }
            else app.exit(0);
        });
        return app.exec();
    }
    // Let stack destructors disable the panel on systemd stop/logout as well.
#ifdef __linux__
    const int signalFd = signalfd(-1, &signalMask, SFD_NONBLOCK | SFD_CLOEXEC);
#else
    int descriptors[2];
    if (pipe(descriptors) != 0) { qCritical() << "Cannot create shutdown pipe"; return 1; }
    for (const int fd : descriptors) { fcntl(fd, F_SETFL, O_NONBLOCK); fcntl(fd, F_SETFD, FD_CLOEXEC); }
    shutdownPipe = descriptors[1];
    struct sigaction action{}; action.sa_handler = requestShutdown; sigemptyset(&action.sa_mask);
    sigaction(SIGTERM, &action, nullptr); sigaction(SIGINT, &action, nullptr);
    const int signalFd = descriptors[0];
#endif
    if (signalFd < 0) { qCritical() << "Cannot watch shutdown signals"; return 1; }
    QSocketNotifier signalNotifier(signalFd, QSocketNotifier::Read);
    QObject::connect(&signalNotifier, &QSocketNotifier::activated, &app, [&] { app.quit(); });
    if (simulator) {
        if (!targetScreen) { qCritical() << "No monitor available"; return 1; }
        Simulator window(ui, backend, appearance, controls, volume);
        window.setScreen(targetScreen);
        const auto area = targetScreen->availableGeometry();
        const QSize size(qMax(502, qMin(2032, int(area.width() * 0.9))), 180);
        window.setGeometry(QRect(area.center() - QPoint(size.width() / 2, size.height() / 2), size));
        if (args.contains("--fullscreen")) window.showFullScreen(); else window.show();
        return app.exec();
    }
#ifndef TOUCHMETER_SIMULATOR_ONLY
    Lease lease;
    TouchInput touch;
    Session session;
    bool idle = false, compositorIdle = false;
    QTimer fireTimer;
    fireTimer.setInterval(33); // Only runs while a visible AI usage screen is burning.
    QTimer touchWakeTimer;
    touchWakeTimer.setSingleShot(true); touchWakeTimer.setInterval(120000);
    auto render = [&] {
        const bool off = idle || !session.allowed();
        volume.setActive(lease.ready() && !off && controls.volumeMode());
        if (!lease.ready()) { fireTimer.stop(); return; }
        QString error;
        QImage image;
        const bool animate = !off && backend.burning() && controls.aiMode() && !controls.providerPicker();
        if (animate && !fireTimer.isActive()) fireTimer.start();
        else if (!animate) fireTimer.stop();
        if (off) { image = QImage(2008, 60, QImage::Format_RGB32); image.fill(Qt::black); }
        else image = ui.render();
        if (image.isNull()) { app.exit(1); return; }
        if (!lease.present(image, &error) && !error.isEmpty()) { qCritical().noquote() << error; app.exit(1); }
        if (!lease.setPower(!off, &error)) { qCritical().noquote() << error; app.exit(1); }
    };
    QObject::connect(&fireTimer, &QTimer::timeout, &app, render);
    QTimer volumeRender;
    volumeRender.setSingleShot(true); volumeRender.setInterval(16);
    QObject::connect(&volumeRender, &QTimer::timeout, &app, render);
    QObject::connect(&volume, &Volume::changed, &app, [&] { if (!volumeRender.isActive()) volumeRender.start(); });
    auto sleep = [&](bool value) {
        idle = value;
        if (idle) controls.cancel();
        controls.setSleeping(idle);
        render();
    };
    QObject::connect(&lease, &Lease::idleChanged, &app, [&](bool value) {
        compositorIdle = value;
        if (!value || !touchWakeTimer.isActive()) sleep(value);
    });
    QObject::connect(&touchWakeTimer, &QTimer::timeout, &app, [&] { if (compositorIdle) sleep(true); });
    QObject::connect(&controls, &Controls::activity, &app, [&] { touchWakeTimer.start(); if (idle) sleep(false); });
    QObject::connect(&session, &Session::changed, &app, [&](bool allowed) {
        controls.cancel();
        touchWakeTimer.stop();
        controls.setSleeping(!allowed || idle);
        render();
    });
    QObject::connect(&controls, &Controls::changed, &app, [&] {
        // Cancel queued audio writes synchronously on navigation, before a deferred render.
        volume.setActive(lease.ready() && !idle && session.allowed() && controls.volumeMode());
        QTimer::singleShot(0, &app, render);
    });
    QObject::connect(&controls, &Controls::functionKey, &app, [&](int number, bool pressed) {
        QString error;
        if (!lease.functionKey(number, pressed, &error)) { qCritical().noquote() << error; app.exit(1); }
    });
    QObject::connect(&touch, &TouchInput::began, &app, [&](double x, double y) { if (session.allowed()) controls.begin(x, y); });
    QObject::connect(&touch, &TouchInput::moved, &app, [&](double x, double y) { if (session.allowed()) controls.move(x, y); });
    QObject::connect(&touch, &TouchInput::ended, &app, [&] { if (session.allowed()) controls.end(); });
    QObject::connect(&touch, &TouchInput::cancelled, &controls, &Controls::cancel);
    QObject::connect(&touch, &TouchInput::failed, &app, [&](const QString &error) { qCritical().noquote() << error; app.exit(1); });
    QObject::connect(&lease, &Lease::available, &app, [&] {
        QString error;
        if (!touch.start(&error)) { qCritical().noquote() << error; app.exit(1); return; }
        qInfo() << "Touch Bar leased; app menu, AI, F keys, volume, and exclusive touch input active";
        QTimer::singleShot(100, &app, render);
    });
    QObject::connect(&lease, &Lease::failed, &app, [&](const QString &message) { qCritical().noquote() << message; app.exit(1); });
    QTimer timer;
    timer.setInterval(15000);
    QObject::connect(&timer, &QTimer::timeout, &app, render); timer.start();
    QObject::connect(&backend, &Backend::changed, &app, [&] { QTimer::singleShot(50, &app, render); });
    QObject::connect(&appearance, &Appearance::changed, &app, [&] { QTimer::singleShot(50, &app, render); });
    QString error;
    if (!lease.start(&error)) { qCritical().noquote() << error; return 1; }
    return app.exec();
#else
    return 1;
#endif
}
