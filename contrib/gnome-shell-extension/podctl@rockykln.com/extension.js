// Puts the podctl bubble on screen. podctl-popup renders every frame and
// hands it over through shared memory; this side only shows it and
// reports the pointer back.

import Clutter from 'gi://Clutter';
import Cogl from 'gi://Cogl';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Config from 'resource:///org/gnome/shell/misc/config.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const BUS_NAME = 'com.rockykln.podctl1.Shell';
const OBJECT_PATH = '/com/rockykln/podctl1/Shell';

const INTERFACE = `
<node>
  <interface name="com.rockykln.podctl1.Shell">
    <!-- Logical pixels throughout: the bubble's size, how much of its
         bottom stays below the screen edge, and the rectangle that takes
         the pointer. Returns the scale of the monitor it opened on. -->
    <method name="Open">
      <arg type="u" direction="in" name="width"/>
      <arg type="u" direction="in" name="height"/>
      <arg type="u" direction="in" name="below"/>
      <arg type="(iiii)" direction="in" name="input"/>
      <arg type="d" direction="out" name="scale"/>
    </method>
    <!-- A memfd holding two frames of slot_size bytes each. -->
    <method name="SetBuffer">
      <arg type="h" direction="in" name="fd"/>
      <arg type="u" direction="in" name="slot_size"/>
    </method>
    <!-- Show the premultiplied BGRA frame in slot. -->
    <method name="Frame">
      <arg type="u" direction="in" name="slot"/>
      <arg type="u" direction="in" name="width"/>
      <arg type="u" direction="in" name="height"/>
      <arg type="u" direction="in" name="stride"/>
    </method>
    <method name="SetCursor">
      <arg type="b" direction="in" name="hand"/>
    </method>
    <method name="Close"/>
    <!-- The frame in slot is on screen and the slot free again. -->
    <signal name="Shown">
      <arg type="u" name="slot"/>
    </signal>
    <!-- kind is motion, leave or click; x and y are logical pixels
         within the bubble. -->
    <signal name="Pointer">
      <arg type="s" name="kind"/>
      <arg type="d" name="x"/>
      <arg type="d" name="y"/>
    </signal>
  </interface>
</node>`;

const SHELL_MAJOR = parseInt(Config.PACKAGE_VERSION.split('.')[0]);

// A view into one slot of the shared buffer. GJS 1.82 (GNOME 48) knows
// g_bytes_new_from_bytes only as a method, 1.90 (GNOME 51) only as a
// static function.
function slice(bytes, offset, length) {
    return GLib.Bytes.new_from_bytes
        ? GLib.Bytes.new_from_bytes(bytes, offset, length)
        : bytes.new_from_bytes(offset, length);
}

class Bubble {
    constructor(service, width, height, input) {
        this._service = service;
        const monitor = Main.layoutManager.currentMonitor ?? Main.layoutManager.primaryMonitor;
        this.scale = global.display.get_monitor_scale(monitor.index);
        // GNOME's own UI factor: 1 when the stage is laid out in logical
        // pixels, the integer monitor scale when it is not.
        const factor = St.ThemeContext.get_for_stage(global.stage).scale_factor;
        this._factor = factor;

        this._content = St.ImageContent.new_with_preferred_size(width * factor, height * factor);
        this._actor = new Clutter.Actor({
            name: 'podctl-bubble',
            content: this._content,
            width: width * factor,
            height: height * factor,
            x: monitor.x + Math.round((monitor.width - width * factor) / 2),
            // The bottom strip stays below the screen edge, as the
            // layer surface does on other desktops: the bubble rises
            // into view inside a window that does not move.
            y: monitor.y + monitor.height - (height - input.below) * factor,
            reactive: false,
        });
        this._actor.set_content_scaling_filters(
            Clutter.ScalingFilter.TRILINEAR, Clutter.ScalingFilter.LINEAR);

        const [ix, iy, iw, ih] = input.rect;
        this._hit = new Clutter.Actor({
            name: 'podctl-input',
            x: ix * factor, y: iy * factor, width: iw * factor, height: ih * factor,
            reactive: true,
        });
        this._actor.add_child(this._hit);
        this._hit.connect('motion-event', (_a, event) => this._pointer('motion', event));
        this._hit.connect('enter-event', (_a, event) => this._pointer('motion', event));
        this._hit.connect('leave-event', () => {
            this._service.emitPointer('leave', -1, -1);
            return Clutter.EVENT_PROPAGATE;
        });
        // The press is taken too, or it reaches whatever lies below,
        // such as the overview, which closes on it.
        this._hit.connect('button-press-event', () => Clutter.EVENT_STOP);
        this._hit.connect('button-release-event', (_a, event) => this._pointer('click', event));

        Main.layoutManager.addTopChrome(this._actor);
        this._quietOsd();
    }

    _pointer(kind, event) {
        const [sx, sy] = event.get_coords();
        const [ok, x, y] = this._actor.transform_stage_point(sx, sy);
        if (ok)
            this._service.emitPointer(kind, x / this._factor, y / this._factor);
        return Clutter.EVENT_STOP;
    }

    // GNOME shows its volume OSD at the bottom centre when the AirPods
    // become the output, right over the bubble. Hold it back while the
    // bubble is up.
    _quietOsd() {
        const osd = Main.osdWindowManager;
        if (!osd)
            return;
        // show() in GNOME 46, showOne()/showAll() later.
        this._osd = {};
        for (const name of ['show', 'showOne', 'showAll']) {
            if (typeof osd[name] !== 'function')
                continue;
            this._osd[name] = osd[name];
            osd[name] = () => {};
        }
        osd.hideAll?.();
    }

    setBuffer(fd, slotSize) {
        this._mapped = GLib.MappedFile.new_from_fd(fd, false);
        this._bytes = this._mapped.get_bytes();
        this._slotSize = slotSize;
    }

    frame(slot, width, height, stride) {
        if (!this._bytes)
            throw new Error('Frame before SetBuffer');
        const bytes = slice(this._bytes, slot * this._slotSize, stride * height);
        const format = Cogl.PixelFormat.BGRA_8888_PRE;
        if (SHELL_MAJOR >= 48) {
            const cogl = global.stage.context.get_backend().get_cogl_context();
            this._content.set_bytes(cogl, bytes, format, width, height, stride);
        } else {
            this._content.set_bytes(bytes, format, width, height, stride);
        }
        // The pixels are copied into a texture by now; the slot is free
        // once the frame has been painted, which also paces the sender.
        const id = global.stage.connect('after-paint', () => {
            global.stage.disconnect(id);
            this._service.emitShown(slot);
        });
        this._actor.queue_redraw();
    }

    setCursor(hand) {
        global.display.set_cursor(hand ? Meta.Cursor.POINTING_HAND : Meta.Cursor.DEFAULT);
        this._hand = hand;
    }

    destroy() {
        if (this._hand)
            global.display.set_cursor(Meta.Cursor.DEFAULT);
        for (const [name, fn] of Object.entries(this._osd ?? {}))
            Main.osdWindowManager[name] = fn;
        Main.layoutManager.removeChrome(this._actor);
        this._actor.destroy();
        this._bytes = null;
        this._mapped = null;
    }
}

class Service {
    constructor() {
        this._dbus = Gio.DBusExportedObject.wrapJSObject(INTERFACE, this);
        this._dbus.export(Gio.DBus.session, OBJECT_PATH);
        this._owner = Gio.bus_own_name_on_connection(
            Gio.DBus.session, BUS_NAME, Gio.BusNameOwnerFlags.NONE, null, null);
        this._bubble = null;
        this._watch = 0;
    }

    OpenAsync([width, height, below, rect], invocation) {
        this._close();
        // A podctl-popup that goes away without closing leaves nothing
        // behind on screen.
        this._watch = Gio.bus_watch_name_on_connection(
            Gio.DBus.session, invocation.get_sender(), Gio.BusNameWatcherFlags.NONE,
            null, () => this._close());
        try {
            this._bubble = new Bubble(this, width, height, {rect, below});
            invocation.return_value(new GLib.Variant('(d)', [this._bubble.scale]));
        } catch (e) {
            this._close();
            invocation.return_error_literal(Gio.DBusError, Gio.DBusError.FAILED, e.message);
        }
    }

    SetBufferAsync([index, slotSize], invocation) {
        try {
            const fds = invocation.get_message().get_unix_fd_list();
            const fd = fds.get(index);
            try {
                this._bubble.setBuffer(fd, slotSize);
            } finally {
                GLib.close(fd);
            }
            invocation.return_value(null);
        } catch (e) {
            invocation.return_error_literal(Gio.DBusError, Gio.DBusError.FAILED, e.message);
        }
    }

    Frame(slot, width, height, stride) {
        this._bubble?.frame(slot, width, height, stride);
    }

    SetCursor(hand) {
        this._bubble?.setCursor(hand);
    }

    Close() {
        this._close();
    }

    emitShown(slot) {
        this._dbus.emit_signal('Shown', new GLib.Variant('(u)', [slot]));
    }

    emitPointer(kind, x, y) {
        this._dbus.emit_signal('Pointer', new GLib.Variant('(sdd)', [kind, x, y]));
    }

    _close() {
        if (this._watch) {
            Gio.bus_unwatch_name(this._watch);
            this._watch = 0;
        }
        this._bubble?.destroy();
        this._bubble = null;
    }

    destroy() {
        this._close();
        Gio.bus_unown_name(this._owner);
        this._dbus.unexport();
    }
}

export default class PodctlExtension extends Extension {
    enable() {
        this._service = new Service();
    }

    disable() {
        this._service?.destroy();
        this._service = null;
    }
}
