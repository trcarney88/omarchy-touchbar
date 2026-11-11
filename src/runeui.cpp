#include "runeui.h"
#include "backend.h"
#include "appearance.h"
#include "controls.h"
#include "volume.h"
#include <QJsonDocument>
#include <QJsonArray>
#include <QDateTime>
#include <QDebug>

extern "C" {
void *touchmeter_ui_new(const void *, size_t);
bool touchmeter_ui_render(void *, const void *, size_t, void *, size_t);
void touchmeter_ui_free(void *);
}

RuneUi::RuneUi(Backend &backend, Appearance &appearance, Controls &controls, Volume *volume)
    : m_backend(backend), m_appearance(appearance), m_controls(controls), m_volume(volume) {
    m_clock.start();
    const auto json = model(); m_ui = touchmeter_ui_new(json.constData(), size_t(json.size()));
}
RuneUi::~RuneUi() { touchmeter_ui_free(m_ui); }
QByteArray RuneUi::model(double time) const {
    const auto colors = m_appearance.colors();
    QJsonObject state{{"providers", QJsonArray::fromVariantList(m_backend.providers())},
        {"selected", m_backend.selectedProvider()}, {"picker", m_controls.providerPicker()},
        {"functions", m_controls.functionsMode()}, {"pressed", m_controls.pressedKey()},
        {"app", m_controls.app()}, {"apps", QJsonArray::fromVariantList(Controls::apps())},
        {"volume", m_volume ? m_volume->fraction() : 0}, {"volume_muted", m_volume && m_volume->muted()},
        {"volume_available", m_volume && m_volume->available()}, {"volume_status", m_volume ? m_volume->status() : "Audio unavailable"},
        {"testing", m_backend.testing()},
        {"fire", m_backend.burning()}, {"burn_minutes", m_backend.burnMinutes()},
        {"fire_trigger_minutes", m_backend.fireTriggerMinutes()},
        {"time", time >= 0 ? time : 1.0 + m_clock.elapsed() / 1000.0},
        {"limits", QJsonArray::fromVariantList(m_backend.limits())}, {"status", m_backend.status()},
        {"stale", m_backend.stale()}, {"shift", int((QDateTime::currentSecsSinceEpoch() / 60) % 15)},
        {"accent_foreground", m_appearance.accentForeground().name()}};
    for (const auto &pair : QList<QPair<QString, QString>>{{"background", "background"}, {"foreground", "foreground"},
        {"accent", "accent"}, {"muted", "muted"}, {"surface", "lighter_background"}, {"warning", "yellow"}, {"danger", "red"}})
        state[pair.first] = colors.value(pair.second).value<QColor>().name();
    return QJsonDocument(state).toJson(QJsonDocument::Compact);
}
QImage RuneUi::render(double time) {
    if (!m_ui) return {};
    QImage image(2008, 60, QImage::Format_RGBA8888);
    const auto json = model(time);
    if (!touchmeter_ui_render(m_ui, json.constData(), size_t(json.size()), image.bits(), size_t(image.sizeInBytes()))) {
        qCritical() << "Rune could not render the Touch Bar"; return {};
    }
    return image;
}
