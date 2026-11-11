#include "lease.h"
#include "drm-lease-client.h"
#include "idle-notify-client.h"
#include "virtual-keyboard-client.h"
#include <QSocketNotifier>
#include <QTimer>
#include <QTransform>
#include <wayland-client.h>
#include <xf86drm.h>
#include <xf86drmMode.h>
#include <drm_fourcc.h>
#include <sys/mman.h>
#include <unistd.h>
#include <cerrno>
#include <cstring>
#include <vector>
#include <xkbcommon/xkbcommon.h>
#include <QSet>
#include <QDebug>
#include <time.h>

struct Lease::State {
    Lease *owner;
    wl_display *display = nullptr;
    wl_registry *registry = nullptr;
    wp_drm_lease_v1 *lease = nullptr;
    wl_seat *seat = nullptr;
    zwp_virtual_keyboard_manager_v1 *keyboardManager = nullptr;
    zwp_virtual_keyboard_v1 *keyboard = nullptr;
    QSet<int> pressedKeys;
    ext_idle_notifier_v1 *idleNotifier = nullptr;
    ext_idle_notification_v1 *idleNotification = nullptr;
    uint32_t idleVersion = 1;
    int fd = -1;
    uint32_t connector = 0, crtc = 0, plane = 0, fb = 0, handle = 0, pitch = 0;
    uint64_t size = 0;
    void *pixels = MAP_FAILED;
    drmModeModeInfo mode{};
    bool requested = false;
    bool powered = false;
    struct Offer {
        State *state;
        wp_drm_lease_device_v1 *device;
        wp_drm_lease_connector_v1 *object;
        uint32_t id = 0;
        QString name;
    };
    std::vector<std::unique_ptr<Offer>> offers;
    std::vector<wp_drm_lease_device_v1 *> devices;

    QString systemError(const QString &what) { return what + ": " + QString::fromLocal8Bit(strerror(errno)); }

    bool configure(QString *error) {
        if (drmSetClientCap(fd, DRM_CLIENT_CAP_ATOMIC, 1) < 0) {
            *error = systemError("Atomic modesetting unavailable"); return false;
        }
        auto conn = drmModeGetConnector(fd, connector);
        if (!conn || conn->count_modes < 1) {
            if (conn) drmModeFreeConnector(conn);
            *error = "Touch Bar has no display mode"; return false;
        }
        mode = conn->modes[0];
        // Refuse to touch anything except this hardware's very narrow panel.
        if (mode.hdisplay != 60 || mode.vdisplay != 2008) {
            drmModeFreeConnector(conn);
            *error = "Refusing a display that is not a 60x2008 Touch Bar"; return false;
        }
        auto resources = drmModeGetResources(fd);
        if (!resources) { drmModeFreeConnector(conn); *error = systemError("No leased resources"); return false; }
        for (int i = 0; i < conn->count_encoders && !crtc; ++i) {
            auto encoder = drmModeGetEncoder(fd, conn->encoders[i]);
            if (!encoder) continue;
            for (int j = 0; j < resources->count_crtcs; ++j)
                if (encoder->possible_crtcs & (1u << j)) { crtc = resources->crtcs[j]; break; }
            drmModeFreeEncoder(encoder);
        }
        int crtcIndex = -1;
        for (int j = 0; j < resources->count_crtcs; ++j) if (resources->crtcs[j] == crtc) crtcIndex = j;
        auto planes = drmModeGetPlaneResources(fd);
        if (planes && crtcIndex >= 0) {
            for (uint32_t i = 0; i < planes->count_planes && !plane; ++i) {
                auto p = drmModeGetPlane(fd, planes->planes[i]);
                if (!p) continue;
                for (uint32_t j = 0; j < p->count_formats; ++j)
                    if ((p->possible_crtcs & (1u << crtcIndex)) && p->formats[j] == DRM_FORMAT_XRGB8888) plane = p->plane_id;
                drmModeFreePlane(p);
            }
        }
        if (planes) drmModeFreePlaneResources(planes);
        drmModeFreeResources(resources);
        drmModeFreeConnector(conn);
        if (!crtc || !plane) { *error = "Lease lacks a compatible CRTC or plane"; return false; }

        drm_mode_create_dumb buffer{};
        buffer.width = 64; buffer.height = mode.vdisplay; buffer.bpp = 32;
        if (drmIoctl(fd, DRM_IOCTL_MODE_CREATE_DUMB, &buffer) < 0) { *error = systemError("Cannot allocate Touch Bar buffer"); return false; }
        handle = buffer.handle; pitch = buffer.pitch; size = buffer.size;
        uint32_t handles[4] = {handle, 0, 0, 0}, pitches[4] = {pitch, 0, 0, 0}, offsets[4] = {};
        if (drmModeAddFB2(fd, 64, mode.vdisplay, DRM_FORMAT_XRGB8888, handles, pitches, offsets, &fb, 0) < 0) {
            *error = systemError("Cannot register framebuffer"); return false;
        }
        drm_mode_map_dumb map{}; map.handle = handle;
        if (drmIoctl(fd, DRM_IOCTL_MODE_MAP_DUMB, &map) < 0) { *error = systemError("Cannot map framebuffer"); return false; }
        pixels = mmap(nullptr, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, map.offset);
        if (pixels == MAP_FAILED) { *error = systemError("Cannot map framebuffer memory"); return false; }
        memset(pixels, 0, size);

        auto request = drmModeAtomicAlloc();
        bool propertiesOk = request != nullptr;
        auto add = [&](uint32_t id, uint32_t type, const char *name, uint64_t value) {
            auto props = drmModeObjectGetProperties(fd, id, type);
            bool found = false;
            if (props) {
                for (uint32_t i = 0; i < props->count_props; ++i) {
                    auto p = drmModeGetProperty(fd, props->props[i]);
                    if (p && !strcmp(p->name, name)) {
                        found = request && drmModeAtomicAddProperty(request, id, p->prop_id, value) >= 0;
                    }
                    if (p) drmModeFreeProperty(p);
                }
                drmModeFreeObjectProperties(props);
            }
            propertiesOk = propertiesOk && found;
        };
        uint32_t blob = 0;
        if (drmModeCreatePropertyBlob(fd, &mode, sizeof(mode), &blob) < 0) {
            if (request) drmModeAtomicFree(request);
            *error = systemError("Cannot create mode blob"); return false;
        }
        add(connector, DRM_MODE_OBJECT_CONNECTOR, "CRTC_ID", crtc);
        add(crtc, DRM_MODE_OBJECT_CRTC, "MODE_ID", blob);
        add(crtc, DRM_MODE_OBJECT_CRTC, "ACTIVE", 0);
        add(plane, DRM_MODE_OBJECT_PLANE, "FB_ID", fb);
        add(plane, DRM_MODE_OBJECT_PLANE, "CRTC_ID", crtc);
        add(plane, DRM_MODE_OBJECT_PLANE, "SRC_X", 0);
        add(plane, DRM_MODE_OBJECT_PLANE, "SRC_Y", 0);
        add(plane, DRM_MODE_OBJECT_PLANE, "SRC_W", uint64_t(mode.hdisplay) << 16);
        add(plane, DRM_MODE_OBJECT_PLANE, "SRC_H", uint64_t(mode.vdisplay) << 16);
        add(plane, DRM_MODE_OBJECT_PLANE, "CRTC_X", 0);
        add(plane, DRM_MODE_OBJECT_PLANE, "CRTC_Y", 0);
        add(plane, DRM_MODE_OBJECT_PLANE, "CRTC_W", mode.hdisplay);
        add(plane, DRM_MODE_OBJECT_PLANE, "CRTC_H", mode.vdisplay);
        int result = propertiesOk ? drmModeAtomicCommit(fd, request, DRM_MODE_ATOMIC_ALLOW_MODESET, nullptr) : -1;
        if (request) drmModeAtomicFree(request);
        drmModeDestroyPropertyBlob(fd, blob);
        if (result < 0) { *error = propertiesOk ? systemError("Cannot activate Touch Bar") : "Missing display properties"; return false; }
        return true;
    }

    static void connectorName(void *data, wp_drm_lease_connector_v1 *, const char *name) { static_cast<Offer *>(data)->name = QString::fromUtf8(name); }
    static void connectorDescription(void *, wp_drm_lease_connector_v1 *, const char *) {}
    static void connectorId(void *data, wp_drm_lease_connector_v1 *, uint32_t id) { static_cast<Offer *>(data)->id = id; }
    static void connectorDone(void *data, wp_drm_lease_connector_v1 *) {
        auto o = static_cast<Offer *>(data); auto s = o->state;
        if (s->requested || o->name != "DSI-1") return;
        s->requested = true; s->connector = o->id;
        auto request = wp_drm_lease_device_v1_create_lease_request(o->device);
        wp_drm_lease_request_v1_request_connector(request, o->object);
        s->lease = wp_drm_lease_request_v1_submit(request);
        static const wp_drm_lease_v1_listener listener = {
            [](void *data, wp_drm_lease_v1 *, int fd) {
                auto s = static_cast<State *>(data); s->fd = fd;
                QString error;
                if (s->configure(&error)) emit s->owner->available();
                else emit s->owner->failed(error);
            },
            [](void *data, wp_drm_lease_v1 *) { emit static_cast<State *>(data)->owner->failed("Touch Bar lease ended; retrying after session resume"); }
        };
        wp_drm_lease_v1_add_listener(s->lease, &listener, s);
    }
    static void connectorWithdrawn(void *, wp_drm_lease_connector_v1 *) {}
    static void deviceFd(void *, wp_drm_lease_device_v1 *, int fd) { close(fd); }
    static void deviceConnector(void *data, wp_drm_lease_device_v1 *device, wp_drm_lease_connector_v1 *connector) {
        auto s = static_cast<State *>(data);
        auto offer = std::make_unique<Offer>(); offer->state = s; offer->device = device; offer->object = connector;
        static const wp_drm_lease_connector_v1_listener listener = {connectorName, connectorDescription, connectorId, connectorDone, connectorWithdrawn};
        wp_drm_lease_connector_v1_add_listener(connector, &listener, offer.get());
        s->offers.push_back(std::move(offer));
    }
    static void deviceDone(void *, wp_drm_lease_device_v1 *) {}
    static void deviceReleased(void *, wp_drm_lease_device_v1 *) {}
    void createKeyboard() {
        if (!seat || !keyboardManager || keyboard) return;
        auto context = xkb_context_new(XKB_CONTEXT_NO_FLAGS);
        xkb_rule_names names{}; names.layout = "us";
        auto map = context ? xkb_keymap_new_from_names(context, &names, XKB_KEYMAP_COMPILE_NO_FLAGS) : nullptr;
        char *text = map ? xkb_keymap_get_as_string(map, XKB_KEYMAP_FORMAT_TEXT_V1) : nullptr;
        int fd = text ? memfd_create("touchmeter-keymap", MFD_CLOEXEC) : -1;
        if (fd >= 0) {
            const size_t length = strlen(text) + 1;
            if (write(fd, text, length) == ssize_t(length)) {
                keyboard = zwp_virtual_keyboard_manager_v1_create_virtual_keyboard(keyboardManager, seat);
                zwp_virtual_keyboard_v1_keymap(keyboard, WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1, fd, length);
                zwp_virtual_keyboard_v1_modifiers(keyboard, 0, 0, 0, 0);
            }
            close(fd);
        }
        free(text);
        if (map) xkb_keymap_unref(map);
        if (context) xkb_context_unref(context);
        if (!keyboard) emit owner->failed("Cannot initialize the function-key keyboard");
    }
    void watchIdle() {
        if (!seat || !idleNotifier || idleNotification) return;
        // Version 2 ignores video-player idle inhibitors for OLED protection.
        idleNotification = idleVersion >= 2
            ? ext_idle_notifier_v1_get_input_idle_notification(idleNotifier, 120000, seat)
            : ext_idle_notifier_v1_get_idle_notification(idleNotifier, 120000, seat);
        static const ext_idle_notification_v1_listener listener = {
            [](void *data, ext_idle_notification_v1 *) { emit static_cast<State *>(data)->owner->idleChanged(true); },
            [](void *data, ext_idle_notification_v1 *) { emit static_cast<State *>(data)->owner->idleChanged(false); }
        };
        ext_idle_notification_v1_add_listener(idleNotification, &listener, this);
    }
    static void global(void *data, wl_registry *registry, uint32_t id, const char *interface, uint32_t version) {
        auto s = static_cast<State *>(data);
        if (!strcmp(interface, "wl_seat") && !s->seat) {
            s->seat = static_cast<wl_seat *>(wl_registry_bind(registry, id, &wl_seat_interface, 1));
            s->watchIdle(); s->createKeyboard(); return;
        }
        if (!strcmp(interface, "zwp_virtual_keyboard_manager_v1")) {
            s->keyboardManager = static_cast<zwp_virtual_keyboard_manager_v1 *>(wl_registry_bind(registry, id, &zwp_virtual_keyboard_manager_v1_interface, 1));
            s->createKeyboard(); return;
        }
        if (!strcmp(interface, "ext_idle_notifier_v1")) {
            s->idleVersion = qMin<uint32_t>(version, 2);
            s->idleNotifier = static_cast<ext_idle_notifier_v1 *>(wl_registry_bind(registry, id, &ext_idle_notifier_v1_interface, s->idleVersion));
            s->watchIdle(); return;
        }
        if (strcmp(interface, "wp_drm_lease_device_v1")) return;
        auto device = static_cast<wp_drm_lease_device_v1 *>(wl_registry_bind(registry, id, &wp_drm_lease_device_v1_interface, 1));
        static const wp_drm_lease_device_v1_listener listener = {deviceFd, deviceConnector, deviceDone, deviceReleased};
        wp_drm_lease_device_v1_add_listener(device, &listener, s); s->devices.push_back(device);
    }
    static void globalRemoved(void *, wl_registry *, uint32_t) {}
};

Lease::Lease(QObject *parent) : QObject(parent), s(std::make_unique<State>()) { s->owner = this; }
Lease::~Lease() {
    QString ignoredPower;
    setPower(false, &ignoredPower);
    if (s->keyboard) {
        const auto keys = s->pressedKeys;
        QString ignored;
        for (int number : keys) functionKey(number, false, &ignored);
        zwp_virtual_keyboard_v1_destroy(s->keyboard);
    }
    if (s->keyboardManager) zwp_virtual_keyboard_manager_v1_destroy(s->keyboardManager);
    if (s->idleNotification) ext_idle_notification_v1_destroy(s->idleNotification);
    if (s->idleNotifier) ext_idle_notifier_v1_destroy(s->idleNotifier);
    if (s->seat) wl_seat_destroy(s->seat);
    if (s->pixels != MAP_FAILED) munmap(s->pixels, s->size);
    if (s->fd >= 0) {
        if (s->fb) drmModeRmFB(s->fd, s->fb);
        if (s->handle) { drm_mode_destroy_dumb destroy{}; destroy.handle = s->handle; drmIoctl(s->fd, DRM_IOCTL_MODE_DESTROY_DUMB, &destroy); }
        close(s->fd);
    }
    if (s->lease) wp_drm_lease_v1_destroy(s->lease);
    for (auto &offer : s->offers) wp_drm_lease_connector_v1_destroy(offer->object);
    for (auto device : s->devices) wp_drm_lease_device_v1_destroy(device);
    if (s->registry) wl_registry_destroy(s->registry);
    if (s->display) { wl_display_flush(s->display); wl_display_disconnect(s->display); }
}
bool Lease::start(QString *error) {
    s->display = wl_display_connect(nullptr);
    if (!s->display) { *error = "Cannot connect to Wayland; run inside your desktop session"; return false; }
    s->registry = wl_display_get_registry(s->display);
    static const wl_registry_listener listener = {State::global, State::globalRemoved};
    wl_registry_add_listener(s->registry, &listener, s.get());
    // Keep protocol IO asynchronous so an unavailable compositor cannot hang the UI.
    auto notifier = new QSocketNotifier(wl_display_get_fd(s->display), QSocketNotifier::Read, this);
    connect(notifier, &QSocketNotifier::activated, this, [this, notifier] {
        if (wl_display_dispatch(s->display) < 0) { notifier->setEnabled(false); emit failed("Wayland disconnected"); return; }
        wl_display_flush(s->display);
    });
    wl_display_flush(s->display);
    QTimer::singleShot(10000, this, [this] {
        if (!ready()) emit failed("No Touch Bar lease available; another daemon may be using it");
        else if (!s->idleNotification) emit failed("Compositor lacks idle notifications; refusing to leave the OLED on indefinitely");
        else if (!s->keyboard) emit failed("Compositor lacks virtual keyboard support for function keys");
    });
    return true;
}
bool Lease::ready() const { return s->fd >= 0 && s->pixels != MAP_FAILED; }
bool Lease::setPower(bool on, QString *error) {
    if (!ready() || s->powered == on) return true;
    auto props = drmModeObjectGetProperties(s->fd, s->crtc, DRM_MODE_OBJECT_CRTC);
    auto request = drmModeAtomicAlloc();
    bool added = false;
    if (props) {
        for (uint32_t i = 0; i < props->count_props; ++i) {
            auto property = drmModeGetProperty(s->fd, props->props[i]);
            if (property && !strcmp(property->name, "ACTIVE") && request)
                added = drmModeAtomicAddProperty(request, s->crtc, property->prop_id, on ? 1 : 0) >= 0;
            if (property) drmModeFreeProperty(property);
        }
        drmModeFreeObjectProperties(props);
    }
    const int result = added ? drmModeAtomicCommit(s->fd, request, DRM_MODE_ATOMIC_ALLOW_MODESET, nullptr) : -1;
    if (request) drmModeAtomicFree(request);
    if (result < 0) { *error = s->systemError("Cannot change Touch Bar power"); return false; }
    s->powered = on;
    qInfo() << "Touch Bar panel power:" << (on ? "on" : "off");
    return true;
}
bool Lease::functionKey(int number, bool pressed, QString *error) {
    if (number < 1 || number > 12) { *error = "Only F1 through F12 may be sent"; return false; }
    if (!s->keyboard) { *error = "Function-key keyboard is not ready"; return false; }
    if (pressed == s->pressedKeys.contains(number)) return true;
    timespec now{}; clock_gettime(CLOCK_MONOTONIC, &now);
    const uint32_t code = number <= 10 ? 58 + number : 76 + number;
    // Hyprland merges the physical keyboards' held modifiers with these.
    zwp_virtual_keyboard_v1_modifiers(s->keyboard, 0, 0, 0, 0);
    zwp_virtual_keyboard_v1_key(s->keyboard, uint32_t(now.tv_sec * 1000 + now.tv_nsec / 1000000), code,
                               pressed ? WL_KEYBOARD_KEY_STATE_PRESSED : WL_KEYBOARD_KEY_STATE_RELEASED);
    if (pressed) s->pressedKeys.insert(number); else s->pressedKeys.remove(number);
    if (wl_display_flush(s->display) < 0 && errno != EAGAIN) { *error = "Cannot send function key to compositor"; return false; }
    return true;
}
bool Lease::present(const QImage &image, QString *error) {
    if (!ready()) return false;
    if (image.size() != QSize(2008, 60)) { *error = "Unexpected dashboard image dimensions"; return false; }
    const QImage rotated = image.transformed(QTransform().rotate(90)).convertToFormat(QImage::Format_RGB32);
    for (int y = 0; y < rotated.height(); ++y)
        memcpy(static_cast<char *>(s->pixels) + y * s->pitch, rotated.constScanLine(y), 60 * 4);
    if (drmModeDirtyFB(s->fd, s->fb, nullptr, 0) < 0 && errno != ENOSYS) { *error = s->systemError("Touch Bar update failed"); return false; }
    return true;
}
