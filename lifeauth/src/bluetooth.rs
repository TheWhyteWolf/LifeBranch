// SPDX-License-Identifier: GPL-3.0-or-later
// org.bluez.Agent1: what bluetoothd asks when pairing needs a person. Without
// an agent, devices that want a PIN or a "does this code match?" (keyboards,
// phones) simply fail to pair; blueman-applet used to be the agent, and with
// it gone this is. Registered as the default agent with KeyboardDisplay
// capability, so bluez picks whichever exchange the device supports.
//
// Every question is a lifemenu prompt; a passkey we must show is a lifenote
// notification. Cancel (the device gave up) closes the open prompt.

use crate::prompt::{self, Slot};
use zbus::zvariant::{ObjectPath, OwnedValue};

pub const PATH: &str = "/org/lifebranch/BluetoothAgent";

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
pub enum BtError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Rejected(String),
    Canceled(String),
}

#[derive(Default)]
pub struct Agent {
    slot: Slot,
}

/// A device's name for the prompt: its alias, else its address.
async fn name(conn: &zbus::Connection, device: &ObjectPath<'_>) -> String {
    let get = async {
        let p = zbus::Proxy::new(conn, "org.bluez", device.to_owned(), "org.freedesktop.DBus.Properties").await?;
        let v: OwnedValue = p.call("Get", &("org.bluez.Device1", "Alias")).await?;
        String::try_from(v).map_err(zbus::Error::from)
    };
    get.await.unwrap_or_else(|_| device.as_str().rsplit('/').next().unwrap_or("device").replace("dev_", "").replace('_', ":"))
}

/// A PIN is 1-16 characters; a passkey is a number up to 999999.
pub fn parse_passkey(s: &str) -> Option<u32> {
    s.trim().parse::<u32>().ok().filter(|&n| n <= 999_999)
}

/// Only bluetoothd may ask; see prompt::from_owner.
async fn guard(conn: &zbus::Connection, hdr: &zbus::message::Header<'_>) -> Result<(), BtError> {
    if prompt::from_owner(conn, hdr, "org.bluez").await { Ok(()) } else { Err(BtError::Rejected("not from bluetoothd".into())) }
}

fn rejected() -> BtError {
    BtError::Rejected("declined".into())
}

impl Agent {
    async fn ask(&self, prompt: String, mesg: String) -> Result<String, BtError> {
        let slot = self.slot.clone();
        blocking::unblock(move || prompt::ask(&prompt, &mesg, false, &slot).map(|s| s.to_string()))
            .await
            .ok_or_else(|| BtError::Canceled("dismissed".into()))
    }
    async fn yes(&self, prompt: String, mesg: String, yes: &'static str) -> bool {
        let slot = self.slot.clone();
        blocking::unblock(move || prompt::confirm(&prompt, &mesg, yes, &slot)).await
    }
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    fn release(&self) {}

    async fn request_pin_code(&self, device: ObjectPath<'_>, #[zbus(connection)] conn: &zbus::Connection, #[zbus(header)] hdr: zbus::message::Header<'_>) -> Result<String, BtError> {
        guard(conn, &hdr).await?;
        let n = name(conn, &device).await;
        let pin = self.ask("PIN".into(), format!("Pairing with {n}: enter the PIN shown on it, or one to type on it")).await?;
        let pin = pin.trim().to_string();
        if pin.is_empty() || pin.len() > 16 {
            return Err(rejected());
        }
        Ok(pin)
    }

    async fn display_pin_code(&self, device: ObjectPath<'_>, pincode: String, #[zbus(connection)] conn: &zbus::Connection, #[zbus(header)] hdr: zbus::message::Header<'_>) {
        if guard(conn, &hdr).await.is_err() {
            return;
        }
        let n = name(conn, &device).await;
        prompt::notify(&format!("Pairing with {n}"), &format!("Type {pincode} on {n}, then Enter"));
    }

    async fn request_passkey(&self, device: ObjectPath<'_>, #[zbus(connection)] conn: &zbus::Connection, #[zbus(header)] hdr: zbus::message::Header<'_>) -> Result<u32, BtError> {
        guard(conn, &hdr).await?;
        let n = name(conn, &device).await;
        let s = self.ask("passkey".into(), format!("Pairing with {n}: enter the 6-digit code it shows")).await?;
        parse_passkey(&s).ok_or_else(rejected)
    }

    async fn display_passkey(&self, device: ObjectPath<'_>, passkey: u32, entered: u16, #[zbus(connection)] conn: &zbus::Connection, #[zbus(header)] hdr: zbus::message::Header<'_>) {
        // Called again for each digit typed on a keyboard; say it once.
        if entered == 0 && guard(conn, &hdr).await.is_ok() {
            let n = name(conn, &device).await;
            prompt::notify(&format!("Pairing with {n}"), &format!("Type {passkey:06} on {n}, then Enter"));
        }
    }

    async fn request_confirmation(&self, device: ObjectPath<'_>, passkey: u32, #[zbus(connection)] conn: &zbus::Connection, #[zbus(header)] hdr: zbus::message::Header<'_>) -> Result<(), BtError> {
        guard(conn, &hdr).await?;
        let n = name(conn, &device).await;
        let mesg = format!("Does {n} show {passkey:06}?");
        if self.yes("pair".into(), mesg, "Yes, the codes match").await { Ok(()) } else { Err(rejected()) }
    }

    async fn request_authorization(&self, device: ObjectPath<'_>, #[zbus(connection)] conn: &zbus::Connection, #[zbus(header)] hdr: zbus::message::Header<'_>) -> Result<(), BtError> {
        guard(conn, &hdr).await?;
        let n = name(conn, &device).await;
        if self.yes("pair".into(), format!("{n} wants to pair with this computer"), "Allow").await { Ok(()) } else { Err(rejected()) }
    }

    async fn authorize_service(&self, device: ObjectPath<'_>, uuid: String, #[zbus(connection)] conn: &zbus::Connection, #[zbus(header)] hdr: zbus::message::Header<'_>) -> Result<(), BtError> {
        guard(conn, &hdr).await?;
        let n = name(conn, &device).await;
        let what = service_name(&uuid);
        if self.yes("bluetooth".into(), format!("Let {n} use {what}?"), "Allow").await { Ok(()) } else { Err(rejected()) }
    }

    fn cancel(&self) {
        prompt::cancel(&self.slot);
    }
}

/// The services anyone is likely to be asked about, by their 16-bit UUID.
pub fn service_name(uuid: &str) -> String {
    let short = uuid.get(4..8).unwrap_or("").to_ascii_lowercase();
    match short.as_str() {
        "110a" | "110b" | "110d" => "audio".into(),
        "110c" | "110e" | "110f" => "media controls".into(),
        "1108" | "111e" | "111f" => "phone audio".into(),
        "1105" => "file transfer".into(),
        "1124" => "input (keyboard/mouse)".into(),
        "1115" | "1116" => "network sharing".into(),
        _ => format!("service {uuid}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passkeys_and_services() {
        assert_eq!(parse_passkey(" 012345\n"), Some(12345));
        assert_eq!(parse_passkey("1000000"), None);
        assert_eq!(parse_passkey("abc"), None);
        assert_eq!(service_name("0000110b-0000-1000-8000-00805f9b34fb"), "audio");
        assert_eq!(service_name("00001124-0000-1000-8000-00805f9b34fb"), "input (keyboard/mouse)");
        assert!(service_name("12345678-0000").starts_with("service "));
    }
}
