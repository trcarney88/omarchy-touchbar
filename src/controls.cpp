#include "controls.h"
#include <cmath>

QVariantList Controls::apps() {
    // A single catalog defines the launcher layout and its hardware hit targets.
    const QList<QPair<QString, QString>> entries{{"functions", "F"}, {"ai", "AI"}, {"volume", ""}};
    QVariantList result;
    const double total = entries.size() * 240 + (entries.size() - 1) * 24;
    for (int i = 0; i < entries.size(); ++i)
        result.append(QVariantMap{{"id", entries[i].first}, {"label", entries[i].second},
            {"x", (2008 - total) / 2 + i * 264}, {"width", 240}});
    return result;
}
void Controls::openApp(const QString &id) {
    bool known = id == "menu";
    for (const auto &entry : apps()) if (entry.toMap().value("id").toString() == id) known = true;
    if (!known) return;
    cancel();
    if (m_app == id && !m_providerPicker) return;
    m_app = id; m_providerPicker = false;
    emit changed();
}
int Controls::keyAt(double x, double y) {
    if (!std::isfinite(x) || !std::isfinite(y) || x < 12 || x >= 1876 || y < 6 || y >= 54) return 0;
    const double width = 1864.0 / 12;
    const int index = int((x - 12) / width);
    const double local = x - 12 - index * width;
    return local >= 3 && local < width - 3 ? index + 1 : 0;
}
int Controls::providerAt(double x, double y) const {
    if (!std::isfinite(x) || !std::isfinite(y) || y < 6 || y >= 54) return -1;
    if (x >= 12 && x < 108) return -2;
    if (!m_providerPicker || x < 132 || x >= 1812) return -1;
    const int i = int((x - 132) / 140);
    if (i >= m_providers.size() || x - 132 - i * 140 >= 100) return -1;
    return m_providers[i].toMap().value("enabled").toBool() ? i : -1;
}
QString Controls::actionAt(double x, double y) const {
    if (!std::isfinite(x) || !std::isfinite(y) || y < 6 || y >= 54) return {};
    if (menuMode()) {
        for (const auto &entry : apps()) {
            const auto a = entry.toMap(); const double left = a.value("x").toDouble();
            if (x >= left && x < left + a.value("width").toDouble()) return "app:" + a.value("id").toString();
        }
        return {};
    }
    if (x >= 1888 && x < 1996) return "back";
    if (volumeMode() && m_volumeAvailable && x >= 12 && x < 120) return "mute";
    if (aiMode()) {
        const int provider = providerAt(x, y);
        if (provider == -2) return "providers";
        if (provider >= 0) return "provider:" + QString::number(provider);
    }
    return {};
}
double Controls::volumeAt(double x) { return qBound(0.0, (x - 220.0) / 1460.0, 1.0); }
void Controls::toggleProviders() {
    cancel();
    if (!aiMode()) return;
    m_providerPicker = !m_providerPicker;
    emit changed();
}
void Controls::begin(double x, double y) {
    if (m_contact) return;
    m_contact = true;
    const bool wasSleeping = m_sleeping;
    m_sleeping = false;
    emit activity();
    // The first contact on a blank OLED only wakes it, including on the slider.
    if (wasSleeping) return;
    if (volumeMode() && m_volumeAvailable && std::isfinite(x) && std::isfinite(y)
        && x >= 200 && x < 1700 && y >= 6 && y < 54) {
        m_dragVolume = true; emit volumeRequested(volumeAt(x)); return;
    }
    m_action = actionAt(x, y);
    if (!m_action.isEmpty() || !functionsMode()) return;
    m_pressedKey = keyAt(x, y);
    if (m_pressedKey) { emit functionKey(m_pressedKey, true); emit changed(); }
}
void Controls::move(double x, double y) {
    if (!m_contact) return;
    emit activity();
    if (m_dragVolume) {
        if (!std::isfinite(x) || !std::isfinite(y) || y < 0 || y >= 60) { m_dragVolume = false; return; }
        emit volumeRequested(volumeAt(x)); return; // Pointer capture: never invoke a button mid-drag.
    }
    if (!m_action.isEmpty() && actionAt(x, y) != m_action) m_action.clear();
    if (m_pressedKey && keyAt(x, y) != m_pressedKey) {
        const int key = m_pressedKey; m_pressedKey = 0;
        emit functionKey(key, false); emit changed();
    }
}
void Controls::end() {
    if (!m_contact) return;
    const auto action = m_action;
    cancel();
    if (action == "back") back();
    else if (action.startsWith("app:")) openApp(action.mid(4));
    else if (action == "mute" && m_volumeAvailable) emit muteRequested();
    else if (action == "providers") toggleProviders();
    else if (action.startsWith("provider:")) {
        const int provider = action.mid(9).toInt();
        if (!m_providers.value(provider).toMap().value("enabled").toBool()) return;
        emit providerSelected(provider); m_providerPicker = false; emit changed();
    }
}
void Controls::cancel() {
    const int key = m_pressedKey;
    m_pressedKey = 0; m_contact = false; m_action.clear(); m_dragVolume = false;
    if (key) { emit functionKey(key, false); emit changed(); }
}
