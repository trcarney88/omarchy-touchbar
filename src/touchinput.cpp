#include "touchinput.h"
#include <QDir>
#include <QFile>
#include <QRegularExpression>
#include <QSocketNotifier>
#include <libevdev/libevdev.h>
#include <linux/input.h>
#include <fcntl.h>
#include <unistd.h>
#include <cerrno>
#include <cstring>

struct TouchInput::State {
    int fd = -1, slot = -1, trackingId = -1;
    libevdev *device = nullptr;
    QSocketNotifier *notifier = nullptr;
    int xmin = 0, xmax = 1, ymin = 0, ymax = 1;
    bool ignoreUntilUp = false;
};
TouchInput::TouchInput(QObject *parent) : QObject(parent), s(std::make_unique<State>()) {}
TouchInput::~TouchInput() {
    if (s->notifier) s->notifier->setEnabled(false);
    if (s->device) { libevdev_grab(s->device, LIBEVDEV_UNGRAB); libevdev_free(s->device); }
    if (s->fd >= 0) close(s->fd);
}
QPointF TouchInput::panelPoint(int x, int y, int xmin, int xmax, int ymin, int ymax) {
    if (xmax <= xmin || ymax <= ymin) return {-1, -1};
    // apple_z2 exposes landscape touch coordinates, unlike the portrait DRM mode.
    return {qBound(0.0, double(x - xmin) / (xmax - xmin), 1.0) * 2007,
            qBound(0.0, double(y - ymin) / (ymax - ymin), 1.0) * 59};
}
bool TouchInput::start(QString *error) {
    const QDir inputs("/sys/class/input");
    for (const auto &entry : inputs.entryList({"event*"}, QDir::Dirs)) {
        QFile name(inputs.filePath(entry + "/device/name"));
        QFile phys(inputs.filePath(entry + "/device/phys"));
        if (!name.open(QIODevice::ReadOnly) || !phys.open(QIODevice::ReadOnly)) continue;
        const auto deviceName = QString::fromUtf8(name.read(256)).trimmed();
        if (!QRegularExpression("^MacBookPro[0-9]+,[0-9]+ Touch Bar$").match(deviceName).hasMatch()
            || phys.read(128).trimmed() != "apple_z2") continue;
        const auto path = QFile::encodeName("/dev/input/" + entry);
        s->fd = open(path.constData(), O_RDONLY | O_NONBLOCK | O_CLOEXEC);
        if (s->fd < 0) {
            *error = tr("Touch Bar input permission missing; reinstall Touchmeter's udev rule"); return false;
        }
        const int result = libevdev_new_from_fd(s->fd, &s->device);
        if (result < 0) { *error = QString::fromLocal8Bit(strerror(-result)); return false; }
        if (!libevdev_has_event_code(s->device, EV_ABS, ABS_MT_TRACKING_ID)
            || !libevdev_has_event_code(s->device, EV_ABS, ABS_MT_POSITION_X)
            || !libevdev_has_event_code(s->device, EV_ABS, ABS_MT_POSITION_Y)) {
            *error = tr("Touch Bar does not provide multitouch coordinates"); return false;
        }
        s->xmin = libevdev_get_abs_minimum(s->device, ABS_MT_POSITION_X);
        s->xmax = libevdev_get_abs_maximum(s->device, ABS_MT_POSITION_X);
        s->ymin = libevdev_get_abs_minimum(s->device, ABS_MT_POSITION_Y);
        s->ymax = libevdev_get_abs_maximum(s->device, ABS_MT_POSITION_Y);
        if (s->xmax <= s->xmin || s->ymax <= s->ymin) { *error = tr("Invalid Touch Bar coordinate range"); return false; }
        // An exclusive grab prevents touch events becoming clicks on the laptop.
        const int grabbed = libevdev_grab(s->device, LIBEVDEV_GRAB);
        if (grabbed < 0) { *error = tr("Cannot exclusively grab Touch Bar input"); return false; }
        s->ignoreUntilUp = true;
        s->notifier = new QSocketNotifier(s->fd, QSocketNotifier::Read, this);
        connect(s->notifier, &QSocketNotifier::activated, this, &TouchInput::readEvents);
        report();
        return true;
    }
    *error = tr("Apple silicon Touch Bar input not found"); return false;
}
void TouchInput::readEvents() {
    input_event event{};
    for (;;) {
        const int status = libevdev_next_event(s->device, LIBEVDEV_READ_FLAG_NORMAL, &event);
        if (status == -EAGAIN) return;
        if (status == LIBEVDEV_READ_STATUS_SYNC) {
            emit cancelled(); s->slot = -1; s->trackingId = -1; s->ignoreUntilUp = true;
            while (libevdev_next_event(s->device, LIBEVDEV_READ_FLAG_SYNC, &event) == LIBEVDEV_READ_STATUS_SYNC) {}
            report(); continue;
        }
        if (status < 0) {
            s->notifier->setEnabled(false); emit cancelled();
            emit failed(tr("Touch Bar input disconnected")); return;
        }
        if (event.type == EV_SYN && event.code == SYN_REPORT) report();
    }
}
void TouchInput::report() {
    const int slotCount = libevdev_get_num_slots(s->device);
    int first = -1;
    for (int slot = 0; slot < slotCount; ++slot)
        if (libevdev_get_slot_value(s->device, slot, ABS_MT_TRACKING_ID) >= 0) { first = slot; break; }
    if (s->ignoreUntilUp) { if (first < 0) s->ignoreUntilUp = false; return; }
    if (s->slot >= 0 && libevdev_get_slot_value(s->device, s->slot, ABS_MT_TRACKING_ID) != s->trackingId) {
        s->slot = -1; s->trackingId = -1; emit ended();
        // Any extra fingers must lift before another gesture can start.
        if (first >= 0) s->ignoreUntilUp = true;
        return;
    }
    if (first < 0) return;
    const bool starting = s->slot < 0;
    if (starting) { s->slot = first; s->trackingId = libevdev_get_slot_value(s->device, first, ABS_MT_TRACKING_ID); }
    const auto point = panelPoint(libevdev_get_slot_value(s->device, s->slot, ABS_MT_POSITION_X),
                                 libevdev_get_slot_value(s->device, s->slot, ABS_MT_POSITION_Y),
                                 s->xmin, s->xmax, s->ymin, s->ymax);
    if (starting) emit began(point.x(), point.y());
    else emit moved(point.x(), point.y());
}
