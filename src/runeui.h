#pragma once
#include <QImage>
#include <QJsonObject>
#include <QElapsedTimer>
class Backend;
class Appearance;
class Controls;
class Volume;

class RuneUi {
public:
    RuneUi(Backend &, Appearance &, Controls &, Volume *volume = nullptr);
    ~RuneUi();
    RuneUi(const RuneUi &) = delete;
    RuneUi &operator=(const RuneUi &) = delete;
    bool ready() const { return m_ui != nullptr; }
    QImage render(double time = -1);
private:
    QByteArray model(double time = -1) const;
    Backend &m_backend;
    Appearance &m_appearance;
    Controls &m_controls;
    Volume *m_volume;
    void *m_ui = nullptr;
    QElapsedTimer m_clock;
};
