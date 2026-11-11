#include <QtTest>
#include "../src/backend.h"
#include "../src/controls.h"
#include "../src/touchinput.h"
#include "../src/appearance.h"
#include "../src/session.h"
#include "../src/runeui.h"
#include "../src/testdata.h"
#include "../src/appconfig.h"
#include "../src/volume.h"
#include "../src/simulator.h"
#include <QMouseEvent>
#include <QFocusEvent>
#include <QTemporaryDir>
#include <QSaveFile>
#include <QJsonArray>
class Tests : public QObject {
    Q_OBJECT
private slots:
    void sessionPowerPolicy() {
        const QVariantMap active{{"Active", true}, {"LockedHint", false}, {"State", "active"},
                                 {"Class", "user"}, {"Type", "wayland"}, {"User", 1000}};
        QVERIFY(Session::permits(active, false, true));
        QVERIFY(!Session::permits(active, true, true));
        QVERIFY(!Session::permits(active, false, false));
        QVERIFY(!Session::permits({}, false, true));
        for (const auto &key : {"Active", "LockedHint", "State", "Class", "Type", "User"}) {
            auto missing = active; missing.remove(key);
            QVERIFY2(!Session::permits(missing, false, true), key);
        }
        for (const auto &pair : QList<QPair<QString, QVariant>>{{"Active", false}, {"LockedHint", true},
                {"State", "closing"}, {"Class", "greeter"}, {"Type", "tty"}}) {
            auto denied = active; denied[pair.first] = pair.second;
            QVERIFY(!Session::permits(denied, false, true));
        }
    }
    void valid() {
        QString error;
        auto result = Backend::parseLimits(R"({"ready":true,"limits":[{"label":"5h window","percent":0.25,"resetsAt":"2026-11-07T04:30:00Z"}]})",
            QDateTime::fromString("2026-11-07T02:00:00Z", Qt::ISODate), &error);
        QVERIFY(error.isEmpty()); QCOMPARE(result.size(), 1);
        QCOMPARE(result[0].toMap()["fraction"].toDouble(), 0.25);
        QCOMPARE(result[0].toMap()["countdown"].toString(), "2h 30m");
    }
    void invalid() {
        for (const auto &raw : QList<QByteArray>{"not json", "[]", "{}", R"({"ready":true,"limits":[]})", QByteArray(1024*1024+1, 'x')}) {
            QString error; QVERIFY(Backend::parseLimits(raw, QDateTime::currentDateTimeUtc(), &error).isEmpty()); QVERIFY(!error.isEmpty());
        }
    }
    void malicious() {
        QString error;
        auto result = Backend::parseLimits(R"({"ready":true,"limits":[{"label":"<img src='https://example.invalid/x'>","percent":0.5,"resetsAt":"2026-11-07T04:30:00Z"}]})",
            QDateTime::fromString("2026-11-07T02:00:00Z", Qt::ISODate), &error);
        QVERIFY(result.isEmpty()); QVERIFY(!error.isEmpty());
    }
    void expired() {
        QString error;
        auto result = Backend::parseLimits(R"({"ready":true,"limits":[{"label":"5h window","percent":0.99,"resetsAt":"2026-11-07T01:00:00Z"}]})",
            QDateTime::fromString("2026-11-07T02:00:00Z", Qt::ISODate), &error);
        QVERIFY(result[0].toMap()["expired"].toBool());
        QCOMPARE(result[0].toMap()["fraction"].toDouble(), 0.99);
    }
    void weeklyAndBounds() {
        const auto now = QDateTime::fromString("2026-11-07T02:00:00Z", Qt::ISODate);
        QString error;
        auto result = Backend::parseLimits(R"json({"ready":true,"limits":[{"label":"Weekly (7-day)","percent":1,"resetsAt":"2026-11-11T04:00:00Z"}]})json", now, &error);
        QCOMPARE(result.size(), 1);
        QCOMPARE(result[0].toMap()["countdown"].toString(), "4d 2h");
        for (auto percentage : {"null", "-0.1", "1.1", "\"0.4\""}) {
            auto raw = QByteArray(R"({"ready":true,"limits":[{"label":"5h window","percent":)") + percentage + R"(,"resetsAt":"2026-11-07T04:00:00Z"}]})";
            QVERIFY(Backend::parseLimits(raw, now, &error).isEmpty());
            QVERIFY(!error.isEmpty());
        }
    }
    void toggleDoesNotType() {
        Controls controls;
        QSignalSpy keys(&controls, &Controls::functionKey);
        QVERIFY(controls.menuMode());
        controls.begin(740, 30); // F app launches on release, not press.
        QVERIFY(!controls.functionsMode());
        controls.end();
        QVERIFY(controls.functionsMode());
        controls.begin(1942, 30); controls.end();
        QVERIFY(controls.menuMode());
        controls.begin(1004, 30); controls.end();
        QVERIFY(controls.aiMode());
        QCOMPARE(keys.size(), 0);
        controls.begin(1942, 30); controls.move(1000, 30); controls.end();
        QVERIFY(!controls.functionsMode());
    }
    void allFunctionKeys() {
        Controls controls; controls.openApp("functions");
        QSignalSpy keys(&controls, &Controls::functionKey);
        for (int number = 1; number <= 12; ++number) {
            controls.begin(12 + (number - 0.5) * (1864.0 / 12), 30);
            QCOMPARE(controls.pressedKey(), number);
            controls.end();
            QCOMPARE(controls.pressedKey(), 0);
            QCOMPARE(keys.takeFirst(), QVariantList({number, true}));
            QCOMPARE(keys.takeFirst(), QVariantList({number, false}));
        }
        QCOMPARE(Controls::keyAt(-1, 30), 0);
        QCOMPARE(Controls::keyAt(1942, 30), 0);
        QCOMPARE(Controls::keyAt(100, 0), 0);
    }
    void slidesCancelAndSleep() {
        Controls controls; controls.openApp("functions");
        QSignalSpy keys(&controls, &Controls::functionKey);
        controls.begin(80, 30); controls.move(240, 30); controls.end();
        QCOMPARE(keys.size(), 2);
        QCOMPARE(keys[0], QVariantList({1, true}));
        QCOMPARE(keys[1], QVariantList({1, false}));
        keys.clear(); controls.setSleeping(true);
        controls.begin(80, 30); controls.end();
        QCOMPARE(keys.size(), 0);
        controls.setSleeping(true); controls.begin(1942, 30); controls.end();
        QVERIFY(controls.functionsMode());
        controls.begin(80, 30); controls.cancel();
        QCOMPARE(keys.size(), 2);
    }
    void coordinatesAndTheme() {
        QCOMPARE(TouchInput::panelPoint(0, 0, 0, 2008, 0, 60), QPointF(0, 0));
        QCOMPARE(TouchInput::panelPoint(2008, 60, 0, 2008, 0, 60), QPointF(2007, 59));
        QCOMPARE(TouchInput::panelPoint(10, 10, 10, 10, 0, 60), QPointF(-1, -1));
        const auto colors = Appearance::parseColors("accent = '#112233'\nbackground = \"#445566\"\nred = 'not-a-color'");
        QCOMPARE(colors["accent"].value<QColor>(), QColor("#112233"));
        QCOMPARE(colors["background"].value<QColor>(), QColor("#445566"));
        QCOMPARE(colors["red"].value<QColor>(), QColor("#ff5f56"));
    }
    void realRuneScene() {
        Appearance appearance(true);
        Backend backend({}, true, false);
        Controls controls;
        controls.openApp("ai");
        controls.setProviders(backend.providers());
        RuneUi ui(backend, appearance, controls);
        QVERIFY(ui.ready());
        const auto usage = ui.render();
        QCOMPARE(usage.size(), QSize(2008, 60));
        QCOMPARE(usage.pixelColor(1000, 1), appearance.colors()["background"].value<QColor>());
        controls.begin(60, 30); controls.end();
        QVERIFY(controls.providerPicker());
        const auto picker = ui.render();
        QVERIFY(picker != usage);
        for (int i = 0; i < backend.providers().size(); ++i) {
            bool visible = false;
            // Enabled logos retain their SVG colors, not the desktop accent.
            const auto ink = i == 0 ? QColor("#ffffff") : i == 1 ? QColor("#D97757") : appearance.colors()["muted"].value<QColor>();
            for (int x = 132 + i * 140 + 33; x < 132 + i * 140 + 67; ++x)
                for (int y = 13; y < 47; ++y) if (picker.pixelColor(x, y) == ink) visible = true;
            QVERIFY2(visible, qPrintable(backend.providers()[i].toMap()["id"].toString() + " logo must render in its enabled/disabled color"));
        }
        controls.begin(1942, 30); controls.end();
        QVERIFY(controls.menuMode());
        controls.begin(740, 30); controls.end();
        QVERIFY(controls.functionsMode());
        QVERIFY(!controls.providerPicker());
        QVERIFY(ui.render() != usage);
        QSignalSpy keys(&controls, &Controls::functionKey);
        controls.begin(1700, 30); controls.end();
        QCOMPARE(keys.size(), 2);
        QCOMPARE(keys[0], QVariantList({11, true}));
        QCOMPARE(keys[1], QVariantList({11, false}));
        controls.begin(1942, 30); controls.end();
        QVERIFY(controls.menuMode());
        controls.begin(1004, 30); controls.end();
        QCOMPARE(ui.render(), usage);
    }
    void providerPickerSafety() {
        Backend backend({}, true, false);
        Controls controls; controls.openApp("ai"); controls.setProviders(backend.providers());
        connect(&controls, &Controls::providerSelected, &backend, &Backend::selectProvider);
        QSignalSpy selected(&controls, &Controls::providerSelected);
        QSignalSpy keys(&controls, &Controls::functionKey);
        controls.setSleeping(true);
        controls.begin(60, 30); controls.end();
        QVERIFY(!controls.providerPicker());
        controls.begin(60, 30); controls.move(120, 30); controls.end();
        QVERIFY(!controls.providerPicker());
        controls.begin(60, 30); controls.end();
        QVERIFY(controls.providerPicker());
        // Grok is unconfigured in the demo: gray and non-interactive.
        controls.begin(462, 30); controls.end();
        QVERIFY(controls.providerPicker()); QCOMPARE(selected.size(), 0);
        QVERIFY(!backend.selectProvider(2));
        QVERIFY(!backend.selectProvider(-1)); QVERIFY(!backend.selectProvider(12));
        controls.begin(322, 30); controls.move(500, 30); controls.end();
        QCOMPARE(selected.size(), 0);
        controls.begin(322, 30); controls.end();
        QCOMPARE(backend.selectedProvider(), 1);
        QVERIFY(!controls.providerPicker()); QCOMPARE(selected.size(), 1);
        QCOMPARE(keys.size(), 0);
        // A sign-out between press and release must also prevent selection.
        controls.begin(60, 30); controls.end();
        controls.begin(182, 30);
        auto providers = backend.providers();
        auto first = providers[0].toMap(); first["enabled"] = false; providers[0] = first;
        controls.setProviders(providers); controls.end();
        QCOMPARE(selected.size(), 1);
    }
    void otherProviderLimits() {
        QString error;
        const auto now = QDateTime::fromString("2026-11-07T02:00:00Z", Qt::ISODate);
        const auto limits = Backend::parseLimits(R"json({"ready":true,"limits":[{"label":"Session (5-hour)","percent":0.4,"resetsAt":"2026-11-07T04:00:00Z"},{"label":"Monthly","percent":0.1,"resetsAt":""}]})json", now, &error);
        QVERIFY(error.isEmpty()); QCOMPARE(limits.size(), 2);
        QCOMPARE(limits[0].toMap()["label"].toString(), "5 HOUR");
        QCOMPARE(limits[1].toMap()["countdown"].toString(), "reset unknown");
        QVERIFY(!limits[1].toMap()["expired"].toBool());
    }
    void providerRecordIsolation() {
        QTemporaryDir tmp; QVERIFY(tmp.isValid());
        QDir root(tmp.path());
        QVERIFY(root.mkpath("bin")); QVERIFY(root.mkpath("omarchy/agents/usage"));
        auto write = [&](const QString &path, const QByteArray &bytes) {
            QFile file(tmp.path() + "/" + path);
            return file.open(QIODevice::WriteOnly) && file.write(bytes) == bytes.size();
        };
        QVERIFY(write("bin/omarchy-agent-usage-deepseek", "def api_key(): return 'test-only-credential'\n"));
        const QByteArray record = R"({"ready":true,"limits":[{"label":"Monthly","percent":0.42,"resetsAt":""}]})";
        QVERIFY(write("omarchy/agents/usage/codex.json", record));
        QVERIFY(write("omarchy/agents/usage/deepseek.json", record));
        const auto oldRoot = qgetenv("OMARCHY_PATH"), oldState = qgetenv("XDG_STATE_HOME");
        auto restore = qScopeGuard([&] {
            if (oldRoot.isNull()) qunsetenv("OMARCHY_PATH"); else qputenv("OMARCHY_PATH", oldRoot);
            if (oldState.isNull()) qunsetenv("XDG_STATE_HOME"); else qputenv("XDG_STATE_HOME", oldState);
        });
        qputenv("OMARCHY_PATH", tmp.path().toUtf8()); qputenv("XDG_STATE_HOME", tmp.path().toUtf8());
        Backend backend({}, false, false);
        QTRY_VERIFY_WITH_TIMEOUT(backend.providers()[4].toMap()["enabled"].toBool(), 5000);
        // Cached "ready" and usage cannot enable Codex without its credential.
        QVERIFY(!backend.providers()[0].toMap()["enabled"].toBool());
        QVERIFY(backend.limits().isEmpty()); QVERIFY(!backend.selectProvider(0));
        QVERIFY(backend.selectProvider(4));
        QCOMPARE(backend.selectedProvider(), 4);
        QCOMPARE(backend.limits().size(), 1);
        QCOMPARE(backend.limits()[0].toMap()["fraction"].toDouble(), 0.42);
        QVERIFY(!backend.selectProvider(1)); QCOMPARE(backend.selectedProvider(), 4);
    }
    void testDataOverridesAndRestoresLiveRecords() {
        QTemporaryDir tmp; QVERIFY(tmp.isValid());
        const auto path = tmp.path() + "/codex.json";
        QFile live(path); QVERIFY(live.open(QIODevice::WriteOnly));
        const QByteArray original = R"({"ready":true,"limits":[]})";
        live.write(original); live.close();
        const auto now = QDateTime::currentDateTimeUtc();
        const QJsonObject p{{"enabled", true}, {"ready", true}, {"limitsFetchedAt", double(now.toMSecsSinceEpoch())},
            {"limits", QJsonArray{QJsonObject{{"label", "5h window"}, {"percent", 0.97}, {"resetsAt", now.addSecs(3600).toString(Qt::ISODate)}}}}};
        QJsonObject fixture{{"version", 1}, {"selected", "claude"}, {"mode", "usage"},
            {"providers", QJsonObject{{"claude", p}, {"deepseek", p}}}};
        QString error; QVERIFY(TestData::validate(fixture, &error));
        Backend backend(path, false, false, nullptr, fixture);
        QVERIFY(backend.testing()); QCOMPARE(backend.selectedProvider(), 1);
        QVERIFY(backend.providers()[1].toMap()["enabled"].toBool());
        QVERIFY(!backend.providers()[0].toMap()["enabled"].toBool());
        QCOMPARE(backend.limits()[0].toMap()["fraction"].toDouble(), 0.97);
        QVERIFY(backend.status().startsWith("TEST · ")); QVERIFY(!backend.stale());
        QVERIFY(backend.selectProvider(4));
        QCOMPARE(backend.limits()[0].toMap()["fraction"].toDouble(), 0.97);
        backend.refresh(); backend.reload(); // Must remain injected, not run a collector.
        QVERIFY(backend.status().startsWith("TEST · "));
        auto providers = fixture["providers"].toObject();
        auto stale = p; stale["limitsStale"] = true; providers["claude"] = stale;
        fixture["providers"] = providers; backend.setTestData(fixture);
        QVERIFY(backend.stale());
        backend.setTestData({});
        QVERIFY(!backend.testing()); QCOMPARE(backend.selectedProvider(), 0);
        QVERIFY(!backend.status().startsWith("TEST · "));
        QVERIFY(!backend.providers()[1].toMap()["enabled"].toBool());
        QVERIFY(live.open(QIODevice::ReadOnly)); QCOMPARE(live.readAll(), original);
        QVERIFY(!QFileInfo::exists(tmp.path() + "/claude.json"));
        QVERIFY(!QFileInfo::exists(tmp.path() + "/deepseek.json"));
    }
    void testDataWatcherHandlesAtomicReplaceAndRemoval() {
        QTemporaryDir tmp; QVERIFY(tmp.isValid());
        const auto path = tmp.path() + "/fixture.json";
        TestData data(path, true);
        QSignalSpy changes(&data, &TestData::changed);
        const QByteArray fixture = R"({"version":1,"selected":"claude","mode":"providers","providers":{"claude":{"enabled":true,"ready":true,"limits":[]}}})";
        auto replace = [&](const QByteArray &bytes) {
            QSaveFile file(path);
            return file.open(QIODevice::WriteOnly) && file.write(bytes) == bytes.size() && file.commit();
        };
        QVERIFY(replace(fixture));
        QTRY_COMPARE_WITH_TIMEOUT(changes.size(), 1, 2000);
        QCOMPARE(data.current()["selected"].toString(), "claude");
        QVERIFY(replace("not-json"));
        QTRY_COMPARE_WITH_TIMEOUT(changes.size(), 2, 2000);
        QVERIFY(data.current().isEmpty()); QVERIFY(!data.error().isEmpty());
        QVERIFY(replace(fixture));
        QTRY_COMPARE_WITH_TIMEOUT(changes.size(), 3, 2000);
        QVERIFY(QFile::remove(path));
        QTRY_COMPARE_WITH_TIMEOUT(changes.size(), 4, 2000);
        QVERIFY(data.current().isEmpty()); QVERIFY(data.error().isEmpty());
    }
    void simulatorOverrideWatchersRemainIndependent() {
        QTemporaryDir tmp; QVERIFY(tmp.isValid());
        const auto physicalPath = tmp.path() + "/touchmeter-test.json";
        const auto simulatorPath = tmp.path() + "/touchmeter-simulator-test.json";
        TestData physical(physicalPath, true), simulator(simulatorPath, true);
        QSignalSpy physicalChanges(&physical, &TestData::changed);
        QSignalSpy simulatorChanges(&simulator, &TestData::changed);
        auto replace = [&](const QString &path, const QJsonObject &data) {
            QSaveFile file(path); const auto bytes = QJsonDocument(data).toJson();
            return file.open(QIODevice::WriteOnly) && file.write(bytes) == bytes.size() && file.commit();
        };
        QString error;
        const auto launch = TestData::scenario("volume-muted", &error);
        Backend backend({}, true, false, nullptr, launch);
        Volume volume(true, "/nonexistent/touchmeter-wpctl-must-not-run");
        connect(&simulator, &TestData::changed, &backend, [&](const QJsonObject &data) {
            const auto effective = data.isEmpty() ? launch : data;
            backend.setTestData(effective);
            volume.setSimulation(true, effective.value("volume").toObject());
        });
        QVERIFY(replace(physicalPath, TestData::scenario("signed-out", &error)));
        QTRY_COMPARE_WITH_TIMEOUT(physicalChanges.size(), 1, 2000);
        QCOMPARE(simulatorChanges.size(), 0); QVERIFY(backend.testing());
        QVERIFY(replace(simulatorPath, TestData::scenario("fire", &error)));
        QTRY_VERIFY_WITH_TIMEOUT(backend.burning(), 2000);
        QCOMPARE(physicalChanges.size(), 1); QVERIFY(volume.simulated());
        QVERIFY(QFile::remove(simulatorPath));
        QTRY_COMPARE_WITH_TIMEOUT(simulatorChanges.size(), 2, 2000);
        QVERIFY(!backend.burning()); QVERIFY(backend.testing()); QVERIFY(volume.muted());
        QCOMPARE(physicalChanges.size(), 1);
        QVERIFY(replace(simulatorPath, TestData::scenario("fire", &error)));
        QTRY_VERIFY_WITH_TIMEOUT(backend.burning(), 2000);
        QFile corrupt(simulatorPath); QVERIFY(corrupt.open(QIODevice::WriteOnly | QIODevice::Truncate));
        corrupt.write("invalid"); corrupt.close();
        QTRY_COMPARE_WITH_TIMEOUT(simulatorChanges.size(), 4, 2000);
        QVERIFY(backend.testing()); QVERIFY(!backend.burning()); QVERIFY(volume.simulated()); QVERIFY(volume.muted());
    }
    void burnRateUsesMeasuredConsumptionAndWindowIdentity() {
        BurnRate rate;
        const auto base = QDateTime::fromString("2026-11-10T12:00:00Z", Qt::ISODate).toMSecsSinceEpoch();
        auto limits = [&](double used, int resetMinutes = 120) {
            return QVariantList{QVariantMap{{"window", "5h window"}, {"fraction", used},
                {"resetsAt", QDateTime::fromMSecsSinceEpoch(base + resetMinutes * 60000).toUTC().toString(Qt::ISODate)}}};
        };
        auto sample = [&](double used, int minutes, bool usable = true, QString account = "one") {
            return rate.observe("codex", account, limits(used), base + minutes * 60000, base + minutes * 60000, usable);
        };
        QCOMPARE(sample(0.5, 0), -1.0); // High percentage alone is not a burn rate.
        QVERIFY(qAbs(sample(0.6, 5) - 20.0) < 0.001);
        QVERIFY(qAbs(sample(0.6, 5) - 20.0) < 0.001); // Cached poll cannot create a new sample.
        QCOMPARE(rate.observe("claude", "one", limits(0.6), base + 5 * 60000, base + 5 * 60000, true), -1.0);
        QCOMPARE(sample(0.7, 6, true, "two"), -1.0); // Account switch.
        QCOMPARE(sample(0.2, 7, true, "two"), -1.0); // Refill/falling percentage.
        QCOMPARE(sample(0.9, 8, false, "two"), -1.0); // Stale, missing, or signed out.
        QCOMPARE(sample(0.95, 9, true, "two"), -1.0); // Needs a fresh baseline after stale.
        QCOMPARE(rate.observe("codex", "two", limits(0.96, 240), base + 10 * 60000, base + 10 * 60000, true), -1.0); // Different reset.
        rate.clear();
        QCOMPARE(sample(0.8, 0), -1.0);
        QCOMPARE(rate.observe("codex", "one", limits(0.81), base + 30000, base + 30000, true), -1.0); // Too short.
        rate.clear(); sample(0.8, 0);
        QVERIFY(sample(0.8, 5) < 0); // Flat/idle usage.
        rate.clear(); sample(0.8, 0);
        QVERIFY(sample(0.81, 5) > 30); // Slow consumption, even near the limit.
        rate.clear(); sample(0.8, 0);
        QCOMPARE(rate.observe("codex", "one", limits(0.9), base, base + 16 * 60000, true), -1.0); // Old sample.
        rate.clear();
        auto beforeRefill = limits(0.5, 10);
        rate.observe("codex", "one", beforeRefill, base, base, true);
        beforeRefill = limits(0.6, 10);
        QCOMPARE(rate.observe("codex", "one", beforeRefill, base + 5 * 60000, base + 5 * 60000, true), -1.0); // Resets before ETA.
    }
    void fireIsAnimatedAndOnlyOnFreshAiUsage() {
        Appearance appearance(true);
        QTemporaryDir tmp; QVERIFY(tmp.isValid());
        const auto now = QDateTime::currentDateTimeUtc();
        QJsonObject p{{"enabled", true}, {"ready", true}, {"burnMinutes", 20}, {"limitsFetchedAt", double(now.toMSecsSinceEpoch())},
            {"limits", QJsonArray{QJsonObject{{"label", "5h window"}, {"percent", 0.7}, {"resetsAt", now.addSecs(3600).toString(Qt::ISODate)}}}}};
        QJsonObject fixture{{"version", 1}, {"selected", "claude"}, {"mode", "usage"}, {"providers", QJsonObject{{"claude", p}}}};
        Backend backend(tmp.path() + "/codex.json", false, false, nullptr, fixture);
        Controls controls; controls.openApp("ai"); controls.setProviders(backend.providers());
        RuneUi ui(backend, appearance, controls); QVERIFY(ui.ready());
        QVERIFY(backend.burning()); QCOMPARE(backend.burnMinutes(), 20.0);
        backend.setFireTriggerMinutes(10); QVERIFY(!backend.burning());
        backend.setFireTriggerMinutes(20); QVERIFY(backend.burning());
        backend.setFireTriggerMinutes(0); QCOMPARE(backend.fireTriggerMinutes(), 20);
        backend.setFireTriggerMinutes(30);
        const auto first = ui.render(1.0), second = ui.render(1.25);
        QVERIFY(!first.isNull()); QVERIFY(first != second);
        QCOMPARE(first, ui.render(1.0)); // Fixed seed/time gives reproducible previews.
        controls.openApp("functions");
        QCOMPARE(ui.render(1.0), ui.render(1.25)); // Never animate F keys.
        controls.openApp("ai"); controls.toggleProviders();
        QCOMPARE(ui.render(1.0), ui.render(1.25)); // Keep the picker quiet/readable.
        controls.toggleProviders();
        p["limitsStale"] = true; fixture["providers"] = QJsonObject{{"claude", p}}; backend.setTestData(fixture);
        QVERIFY(!backend.burning()); QCOMPARE(ui.render(1.0), ui.render(1.25));
        p["limitsStale"] = false; p["burnMinutes"] = 31; fixture["providers"] = QJsonObject{{"claude", p}}; backend.setTestData(fixture);
        QVERIFY(!backend.burning());
        p["burnMinutes"] = 30; fixture["providers"] = QJsonObject{{"claude", p}}; backend.setTestData(fixture);
        QVERIFY(backend.burning());
        p["limitsFetchedAt"] = 1e100; fixture["providers"] = QJsonObject{{"claude", p}}; backend.setTestData(fixture);
        QVERIFY(!backend.burning()); QVERIFY(backend.stale());
        p["limitsFetchedAt"] = double(now.toMSecsSinceEpoch());
        p["enabled"] = false; fixture["providers"] = QJsonObject{{"claude", p}}; backend.setTestData(fixture);
        QVERIFY(!backend.burning());
        backend.setTestData({}); QVERIFY(!backend.burning());
    }
    void configurationReloadsAndKeepsLastValidValue() {
        QTemporaryDir tmp; QVERIFY(tmp.isValid());
        const auto path = tmp.path() + "/config.json";
        AppConfig config(path, true);
        QCOMPARE(config.fireTriggerMinutes(), 30);
        QSignalSpy changes(&config, &AppConfig::changed);
        auto replace = [&](const QByteArray &bytes) {
            QSaveFile file(path);
            return file.open(QIODevice::WriteOnly) && file.write(bytes) == bytes.size() && file.commit();
        };
        QVERIFY(replace(R"({"version":1,"fire":{"triggerMinutes":60}})"));
        QTRY_COMPARE_WITH_TIMEOUT(config.fireTriggerMinutes(), 60, 2000);
        QCOMPARE(changes.size(), 1);
        QVERIFY(replace("bad-json"));
        QTest::qWait(600);
        QCOMPARE(config.fireTriggerMinutes(), 60); QCOMPARE(changes.size(), 1);
        QVERIFY(replace(R"({"version":1,"fire":{"triggerMinutes":15}})"));
        QTRY_COMPARE_WITH_TIMEOUT(config.fireTriggerMinutes(), 15, 2000);
        QCOMPARE(changes.size(), 2);
        QVERIFY(QFile::remove(path));
        QTRY_COMPARE_WITH_TIMEOUT(config.fireTriggerMinutes(), 30, 2000);
        QCOMPARE(changes.size(), 3);
    }
    void configurationLiveChangesUpdateInjectedFire() {
        QTemporaryDir tmp; QVERIFY(tmp.isValid());
        const auto path = tmp.path() + "/config.json";
        AppConfig config(path, true);
        const auto now = QDateTime::currentDateTimeUtc();
        const QJsonObject p{{"enabled", true}, {"ready", true}, {"burnMinutes", 45}, {"limitsFetchedAt", double(now.toMSecsSinceEpoch())},
            {"limits", QJsonArray{QJsonObject{{"label", "5h window"}, {"percent", 0.5}, {"resetsAt", now.addSecs(7200).toString(Qt::ISODate)}}}}};
        const QJsonObject fixture{{"version", 1}, {"selected", "claude"}, {"mode", "usage"}, {"providers", QJsonObject{{"claude", p}}}};
        Backend backend(tmp.path() + "/codex.json", false, false, nullptr, fixture);
        connect(&config, &AppConfig::changed, &backend, [&] { backend.setFireTriggerMinutes(config.fireTriggerMinutes()); });
        QVERIFY(!backend.burning());
        QSaveFile file(path); QVERIFY(file.open(QIODevice::WriteOnly));
        file.write(R"({"version":1,"fire":{"triggerMinutes":60}})"); QVERIFY(file.commit());
        QTRY_VERIFY_WITH_TIMEOUT(backend.burning(), 2000);
        QCOMPARE(backend.burnMinutes(), 45.0);
        QVERIFY(QFile::remove(path));
        QTRY_VERIFY_WITH_TIMEOUT(!backend.burning(), 2000);
        QCOMPARE(backend.burnMinutes(), 45.0); // Preferences never alter injected measurements.
    }
    void appMenuAndBackAreSafeForEveryApp() {
        Controls controls;
        QSignalSpy keys(&controls, &Controls::functionKey);
        QSignalSpy levels(&controls, &Controls::volumeRequested);
        for (const auto &entry : Controls::apps()) {
            const auto a = entry.toMap(); const double x = a["x"].toDouble() + a["width"].toDouble() / 2;
            QVERIFY(controls.menuMode());
            controls.begin(x, 30); controls.move(50, 30); controls.end();
            QVERIFY(controls.menuMode()); // Slide off never opens another app.
            controls.setSleeping(true); controls.begin(x, 30); controls.end();
            QVERIFY(controls.menuMode());
            controls.begin(x, 30); controls.end();
            QCOMPARE(controls.app(), a["id"].toString());
            controls.begin(1942, 30); controls.end();
            QVERIFY(controls.menuMode());
        }
        controls.openApp("not-registered"); QVERIFY(controls.menuMode());
        QCOMPARE(keys.size(), 0); QCOMPARE(levels.size(), 0);
        controls.openApp("functions"); controls.begin(80, 30);
        controls.back(); // Navigation must always release a held key.
        QCOMPARE(keys.size(), 2); QCOMPARE(keys.last(), QVariantList({1, false}));
        QVERIFY(controls.menuMode());
    }
    void volumeDragMuteWakeAndCapture() {
        Volume volume(true, "/nonexistent/touchmeter-wpctl-must-not-run"); volume.setActive(true);
        Controls controls; controls.openApp("volume"); controls.setVolumeAvailable(volume.available());
        connect(&controls, &Controls::volumeRequested, &volume, &Volume::setVolume);
        connect(&controls, &Controls::muteRequested, &volume, &Volume::toggleMute);
        QSignalSpy levels(&controls, &Controls::volumeRequested), mutes(&controls, &Controls::muteRequested);
        controls.begin(220, 30); QCOMPARE(volume.fraction(), 0.0);
        controls.move(950, 30); QCOMPARE(volume.fraction(), 0.5);
        controls.move(1680, 30); QCOMPARE(volume.fraction(), 1.0);
        controls.move(1942, 30); controls.end();
        QVERIFY(controls.volumeMode()); QCOMPARE(volume.fraction(), 1.0); // Captured slider cannot hit Back.
        controls.begin(66, 30); QVERIFY(!volume.muted()); controls.end(); QVERIFY(volume.muted());
        controls.begin(950, 30); controls.move(60, 30); controls.end();
        QCOMPARE(volume.fraction(), 0.0); QVERIFY(volume.muted()); QCOMPARE(mutes.size(), 1);
        controls.begin(66, 30); controls.move(200, 30); controls.end();
        QCOMPARE(mutes.size(), 1); // Mute activates only on release inside its original target.
        const auto count = levels.size();
        controls.setSleeping(true); controls.begin(1680, 30); controls.move(950, 30); controls.end();
        QCOMPARE(levels.size(), count); QCOMPARE(volume.fraction(), 0.0);
        controls.begin(950, 30); controls.setSleeping(true); controls.move(1680, 30); controls.end();
        QCOMPARE(volume.fraction(), 0.5); // Cancel on lock/sleep stops the drag.
        controls.setSleeping(false); controls.setVolumeAvailable(false);
        controls.begin(1680, 30); controls.end(); controls.begin(66, 30); controls.end();
        QCOMPARE(volume.fraction(), 0.5); QCOMPARE(mutes.size(), 1);
        volume.setActive(false); volume.setVolume(1.0); volume.toggleMute();
        QCOMPARE(volume.fraction(), 0.5); QVERIFY(volume.muted());
    }
    void volumeParsing() {
        double level; bool muted;
        QVERIFY(Volume::parse("Volume: 0.65\n", &level, &muted)); QCOMPARE(level, 0.65); QVERIFY(!muted);
        QVERIFY(Volume::parse("Volume: 0.40 [MUTED]\n", &level, &muted)); QCOMPARE(level, 0.4); QVERIFY(muted);
        QVERIFY(Volume::parse("Volume: 1.50", &level, &muted)); QCOMPARE(level, 1.0);
        for (const auto &data : QList<QByteArray>{"", "bad", "Volume: -1", "Volume: nan", "Volume: 0,5", "Volume: 0.5 garbage", QByteArray(257, 'a')})
            QVERIFY(!Volume::parse(data, &level, &muted));
    }
    void volumeBackendSerializesCoalescesAndFollowsExternalChanges() {
        QTemporaryDir tmp; QVERIFY(tmp.isValid());
        const auto state = tmp.path() + "/state.json", log = tmp.path() + "/calls", program = tmp.path() + "/wpctl";
        auto writeState = [&](double level, bool muted) {
            QSaveFile file(state); if (!file.open(QIODevice::WriteOnly)) return false;
            file.write(QJsonDocument(QJsonObject{{"volume", level}, {"muted", muted}}).toJson()); return file.commit();
        };
        auto readState = [&] { QFile file(state); return file.open(QIODevice::ReadOnly) ? QJsonDocument::fromJson(file.readAll()).object() : QJsonObject(); };
        auto readLog = [&] { QFile file(log); return file.open(QIODevice::ReadOnly) ? file.readAll() : QByteArray(); };
        QVERIFY(writeState(0.5, false));
        QFile script(program); QVERIFY(script.open(QIODevice::WriteOnly));
        script.write(R"py(#!/usr/bin/python3
import json, sys
from pathlib import Path
root = Path(__file__).parent
args = sys.argv[1:]
with (root / 'calls').open('a') as f: f.write(json.dumps(args) + '\n')
if (root / 'fail').exists(): sys.exit(1)
state = json.loads((root / 'state.json').read_text())
if args[0] == 'get-volume':
    print('Volume: %.3f%s' % (state['volume'], ' [MUTED]' if state['muted'] else ''))
elif args[0] == 'set-volume':
    assert args[1:4] == ['--limit', '1.0', '@DEFAULT_AUDIO_SINK@']
    assert 0 <= float(args[4]) <= 1
    state['volume'] = float(args[4])
    (root / 'state.json').write_text(json.dumps(state))
elif args[0] == 'set-mute':
    state['muted'] = args[2] == '1'
    (root / 'state.json').write_text(json.dumps(state))
else: sys.exit(2)
)py");
        script.close(); QVERIFY(script.setPermissions(QFileDevice::ReadOwner | QFileDevice::WriteOwner | QFileDevice::ExeOwner));
        Volume volume(false, program);
        volume.setActive(true);
        QTRY_VERIFY_WITH_TIMEOUT(volume.available(), 2000); QCOMPARE(volume.fraction(), 0.5);
        volume.setVolume(0.2); volume.setVolume(0.4); volume.setVolume(0.8);
        QTRY_COMPARE_WITH_TIMEOUT(readState()["volume"].toDouble(), 0.8, 2000);
        QCOMPARE(readLog().count("set-volume"), 1); // Rapid motions collapse into the latest target.
        QVERIFY(writeState(0.33, true));
        QTRY_COMPARE_WITH_TIMEOUT(volume.fraction(), 0.33, 2500); QVERIFY(volume.muted());
        volume.toggleMute(); QTRY_VERIFY_WITH_TIMEOUT(!readState()["muted"].toBool(), 2000);
        volume.setVolume(2.0); QTRY_COMPARE_WITH_TIMEOUT(readState()["volume"].toDouble(), 1.0, 2000);
        volume.setVolume(0.1); volume.setActive(false);
        QTest::qWait(60); QCOMPARE(readState()["volume"].toDouble(), 1.0); // No queued writes after lock/navigation.
        const auto calls = readLog(); QTest::qWait(1100); QCOMPARE(readLog(), calls);
        QFile fail(tmp.path() + "/fail"); QVERIFY(fail.open(QIODevice::WriteOnly)); fail.close();
        volume.setActive(true); QTest::qWait(150); QVERIFY(!volume.available());
        QVERIFY(fail.remove()); QTRY_VERIFY_WITH_TIMEOUT(volume.available(), 2500);
        volume.setActive(false);
    }
    void realRuneVolumeAndMenu() {
        Appearance appearance(true); Backend backend({}, true, false); Controls controls;
        Volume volume(true, "/nonexistent/touchmeter-wpctl-must-not-run"); volume.setActive(true);
        RuneUi ui(backend, appearance, controls, &volume); QVERIFY(ui.ready());
        const auto menu = ui.render(1.0); QVERIFY(!menu.isNull());
        controls.openApp("volume");
        const auto unmuted = ui.render(1.0); QVERIFY(unmuted != menu);
        volume.setVolume(0.2); QVERIFY(ui.render(1.0) != unmuted);
        volume.toggleMute(); const auto muted = ui.render(1.0); QVERIFY(muted != unmuted);
        volume.setSimulation(true, QJsonObject{{"available", false}}); QVERIFY(ui.render(1.0) != muted);
        controls.back(); QCOMPARE(ui.render(1.0), menu);
    }
    void simulatorCoordinateMappingPreservesAspectRatio() {
        for (const auto &size : {QSize(1600, 180), QSize(3840, 2160), QSize(502, 120)}) {
            const auto rect = Simulator::barRect(size);
            QVERIFY(rect.width() > 0); QVERIFY(qAbs(rect.width() / rect.height() - 2008.0 / 60.0) < 0.001);
            const auto center = Simulator::panelPoint(rect.center(), size);
            QVERIFY(qAbs(center.x() - 1004) < 0.001); QVERIFY(qAbs(center.y() - 30) < 0.001);
            QVERIFY(Simulator::panelPoint({0, 0}, size).y() < 0);
        }
        QCOMPARE(Simulator::panelPoint({0, 0}, QSize()), QPointF(-1, -1));
    }
    void simulatorMouseNavigationDragAndFocusLoss() {
        const bool quit = QGuiApplication::quitOnLastWindowClosed();
        QGuiApplication::setQuitOnLastWindowClosed(false);
        auto restore = qScopeGuard([&] { QGuiApplication::setQuitOnLastWindowClosed(quit); });
        Appearance appearance(true);
        QString error; const auto fixture = TestData::scenario("menu", &error); QVERIFY(error.isEmpty());
        Backend backend({}, true, false, nullptr, fixture);
        Controls controls; controls.setProviders(backend.providers());
        Volume volume(true, "/nonexistent/touchmeter-wpctl-must-not-run"); controls.setVolumeAvailable(volume.available());
        connect(&controls, &Controls::volumeRequested, &volume, &Volume::setVolume);
        connect(&controls, &Controls::muteRequested, &volume, &Volume::toggleMute);
        RuneUi ui(backend, appearance, controls, &volume); QVERIFY(ui.ready());
        Simulator window(ui, backend, appearance, controls, volume);
        window.resize(1600, 180); window.show();
        QTRY_VERIFY_WITH_TIMEOUT(window.isExposed(), 2000);
        auto point = [&](double x, double y = 30) {
            const auto rect = Simulator::barRect(window.size());
            return QPoint(qRound(rect.x() + x * rect.width() / 2008), qRound(rect.y() + y * rect.height() / 60));
        };
        auto click = [&](double x) { QTest::mouseClick(&window, Qt::LeftButton, Qt::NoModifier, point(x)); };
        QSignalSpy keys(&controls, &Controls::functionKey);
        click(740); QVERIFY(controls.functionsMode());
        QTest::mousePress(&window, Qt::LeftButton, Qt::NoModifier, point(1700));
        QCOMPARE(controls.pressedKey(), 11); QVERIFY(window.title().contains("F11 held"));
        QFocusEvent lost(QEvent::FocusOut); QCoreApplication::sendEvent(&window, &lost);
        QCOMPARE(controls.pressedKey(), 0); QCOMPARE(keys.size(), 2);
        QTest::mouseRelease(&window, Qt::LeftButton, Qt::NoModifier, point(1700));
        click(1942); QVERIFY(controls.menuMode());
        click(1268); QVERIFY(controls.volumeMode()); QVERIFY(volume.simulated());
        QTest::mousePress(&window, Qt::LeftButton, Qt::NoModifier, point(950));
        QVERIFY(qAbs(volume.fraction() - 0.5) < 0.002);
        const QPointF at = point(1500);
        QMouseEvent drag(QEvent::MouseMove, at, at, Qt::NoButton, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(&window, &drag);
        QVERIFY(qAbs(volume.fraction() - (1500.0 - 220) / 1460) < 0.002);
        QTest::mouseRelease(&window, Qt::LeftButton, Qt::NoModifier, point(1500));
        click(66); QVERIFY(volume.muted());
        QTest::mouseClick(&window, Qt::LeftButton, Qt::NoModifier, QPoint(5, 5));
        QVERIFY(controls.volumeMode()); QVERIFY(volume.muted()); // Letterboxing isn't touch input.
        click(1942); QVERIFY(controls.menuMode());
        click(1004); QVERIFY(controls.aiMode());
        QTest::keyClick(&window, Qt::Key_M); QVERIFY(controls.menuMode());
        QTest::keyClick(&window, Qt::Key_F11);
        QTRY_COMPARE_WITH_TIMEOUT(window.windowState(), Qt::WindowFullScreen, 2000);
        QTest::keyClick(&window, Qt::Key_Escape);
        QTRY_COMPARE_WITH_TIMEOUT(window.windowState(), Qt::WindowNoState, 2000);
        QTest::keyClick(&window, Qt::Key_Escape);
        QTRY_VERIFY_WITH_TIMEOUT(!window.isVisible(), 2000);
    }
    void fictionalSimulatorClockSupportsFireAndSignedOut() {
        QString error; auto fixture = TestData::scenario("fire", &error);
        QVERIFY(error.isEmpty());
        Backend backend({}, true, false, nullptr, fixture);
        QVERIFY(backend.testing()); QVERIFY(backend.burning());
        fixture = TestData::scenario("signed-out", &error); backend.setTestData(fixture);
        QVERIFY(backend.limits().isEmpty()); QVERIFY(!backend.burning());
        QVERIFY(backend.status().contains("Sign-in required"));
    }
};
QTEST_MAIN(Tests)
#include "touchmeter_tests.moc"
