#include "appearance.h"
#include <QDir>
#include <QFile>
#include <QRegularExpression>
#include <QJsonDocument>
#include <QGuiApplication>
#include <QLocale>

Appearance::Appearance(bool demo, const QString &languageOverride, QObject *parent)
    : QTranslator(parent), m_demo(demo), m_languageOverride(languageOverride) {
    m_rounding.setProgram("hyprctl");
    m_rounding.setArguments({"-j", "getoption", "decoration:rounding"});
    connect(&m_rounding, &QProcess::finished, this, [this] {
        const auto json = QJsonDocument::fromJson(m_rounding.readAllStandardOutput()).object();
        m_rounding.readAllStandardError();
        const int radius = qBound(0, json.value("int").toInt(), 30);
        if (radius != m_radius) { m_radius = radius; emit changed(); }
    });
    reload();
    // Polling follows atomic file and theme-symlink replacement without stale watches.
    m_poll.setInterval(2000);
    connect(&m_poll, &QTimer::timeout, this, &Appearance::reload);
    m_poll.start();
}
QString Appearance::translate(const char *, const char *source, const char *, int) const {
    return m_strings.value(QString::fromUtf8(source)).toString();
}
QVariantMap Appearance::parseColors(const QByteArray &data) {
    QVariantMap colors{{"accent", QColor("#FFD60A")}, {"background", QColor("#101315")},
                       {"foreground", QColor("#cacccc")}, {"muted", QColor("#707880")},
                       {"lighter_background", QColor("#272727")}, {"red", QColor("#ff5f56")},
                       {"yellow", QColor("#ffc163")}};
    const auto text = QString::fromUtf8(data.left(65536));
    auto matches = QRegularExpression("(?m)^([a-z_]+)\\s*=\\s*[\"'](#[0-9a-fA-F]{6})[\"']").globalMatch(text);
    while (matches.hasNext()) {
        const auto match = matches.next();
        if (colors.contains(match.captured(1))) colors[match.captured(1)] = QColor(match.captured(2));
    }
    return colors;
}
QColor Appearance::accentForeground() const {
    // A contrasting theme color makes the toggle readable in light and dark themes.
    const auto background = m_colors.value("background").value<QColor>();
    const auto foreground = m_colors.value("foreground").value<QColor>();
    auto contrast = [this](const QColor &color) { return qAbs(color.lightnessF() - m_accent.lightnessF()); };
    return contrast(background) >= contrast(foreground) ? background : foreground;
}
void Appearance::reload() {
    QFile theme(QDir::homePath() + "/.local/state/omarchy/current/theme/colors.toml");
    QVariantMap colors = parseColors({});
    if (!m_demo && theme.open(QIODevice::ReadOnly)) {
        colors = parseColors(theme.read(65536));
    }
    if (!m_demo && m_rounding.state() == QProcess::NotRunning) m_rounding.start();
    QFile file(QDir::homePath() + "/.config/omarchy/language");
    QString language = "en";
    if (file.open(QIODevice::ReadOnly)) {
        const auto code = QString::fromUtf8(file.readLine(64)).trimmed();
        if (QRegularExpression("^[a-z]{2,3}(-[A-Za-z]{2})?$").match(code).hasMatch()) language = code;
    }
    if (QRegularExpression("^[a-z]{2,3}(-[A-Za-z]{2})?$").match(m_languageOverride).hasMatch()) language = m_languageOverride;
    if (language == m_language && colors == m_colors) return;
    m_language = language; m_colors = colors; m_accent = colors["accent"].value<QColor>(); m_strings = {};
    QStringList codes{"en", language.section('-', 0, 0), language}; codes.removeDuplicates();
    for (const auto &code : codes) {
        QFile catalog(":/i18n/" + code + ".json");
        if (!catalog.open(QIODevice::ReadOnly)) continue;
        const auto strings = QJsonDocument::fromJson(catalog.readAll()).object();
        for (auto it = strings.begin(); it != strings.end(); ++it) if (it.value().isString()) m_strings.insert(it.key(), it.value());
    }
    QLocale locale(language);
    QFile registry(qEnvironmentVariable("OMARCHY_PATH", "/usr/share/omarchy") + "/default/i18n/locales.json");
    if (registry.open(QIODevice::ReadOnly)) {
        const auto name = QJsonDocument::fromJson(registry.readAll()).object().value(language).toObject().value("formatLocale").toString();
        if (!name.isEmpty()) locale = QLocale(name);
    }
    QLocale::setDefault(locale);
    QGuiApplication::setLayoutDirection(locale.textDirection());
    emit changed();
}
