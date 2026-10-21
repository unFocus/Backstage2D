//! Following the desktop's light or dark preference.
//!
//! Plain GTK (no libadwaita) goes dark only through
//! `gtk-application-prefer-dark-theme` or a dark theme name. KDE sets those
//! for GTK apps; GNOME only publishes `color-scheme` through the settings
//! portal, so the editor reads it there itself. See ADR 0001.

use gtk::prelude::*;
use gtk::{gio, glib};

const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS_IFACE: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

/// The portal's `color-scheme`: 0 no preference, 1 prefer dark, 2 prefer
/// light. Anything else is treated as no preference.
pub fn prefers_dark(color_scheme: u32) -> bool {
    color_scheme == 1
}

/// Applies the portal's preference now and whenever it changes. Without a
/// portal (macOS, Windows, a bare session) GTK's own default stays.
pub fn follow_system() {
    let Ok(proxy) = gio::DBusProxy::for_bus_sync(
        gio::BusType::Session,
        gio::DBusProxyFlags::DO_NOT_AUTO_START,
        None,
        PORTAL_NAME,
        PORTAL_PATH,
        SETTINGS_IFACE,
        None::<&gio::Cancellable>,
    ) else {
        return;
    };
    if let Some(scheme) = read_scheme(&proxy) {
        apply(scheme);
    }
    proxy.connect_local("g-signal", false, |args| {
        let signal = args[2].get::<String>().ok()?;
        let params = args[3].get::<glib::Variant>().ok()?;
        if signal == "SettingChanged" {
            let (namespace, key, value) = params.get::<(String, String, glib::Variant)>()?;
            if namespace == NAMESPACE && key == KEY {
                apply(value.get::<u32>()?);
            }
        }
        None
    });
    // The signal handler lives as long as the proxy, which is the app.
    std::mem::forget(proxy);
}

/// `ReadOne` returns `(v)`; the older `Read` wraps the value once more.
fn read_scheme(proxy: &gio::DBusProxy) -> Option<u32> {
    let call = |method| {
        proxy
            .call_sync(
                method,
                Some(&(NAMESPACE, KEY).to_variant()),
                gio::DBusCallFlags::NONE,
                1000,
                None::<&gio::Cancellable>,
            )
            .ok()
    };
    if let Some(reply) = call("ReadOne") {
        return reply.child_value(0).as_variant()?.get::<u32>();
    }
    let reply = call("Read")?;
    reply.child_value(0).as_variant()?.as_variant()?.get::<u32>()
}

fn apply(scheme: u32) {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(prefers_dark(scheme));
    }
}

#[cfg(test)]
mod tests {
    use super::prefers_dark;

    #[test]
    fn only_prefer_dark_turns_dark() {
        assert!(!prefers_dark(0), "no preference");
        assert!(prefers_dark(1), "prefer dark");
        assert!(!prefers_dark(2), "prefer light");
        assert!(!prefers_dark(7), "unknown");
    }
}
