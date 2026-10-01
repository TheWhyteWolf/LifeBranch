// SPDX-License-Identifier: GPL-3.0-or-later
// org.freedesktop.NetworkManager.SecretAgent: what NetworkManager asks when a
// connection needs a secret it doesn't have. A new wifi joined from anywhere,
// an enterprise (802.1X) network's identity and password, an OpenVPN profile's
// password: nm-applet used to be the agent that asked, and with it gone this
// is. Registered with the VPN_HINTS capability, so NM asks us for a VPN's
// secrets by name instead of running the plugin's own (GTK) dialog.
//
// Only asks when NM says a person may be asked (ALLOW_INTERACTION); otherwise
// it answers NoSecrets at once, as the protocol wants. Nothing is stored here:
// NM keeps system-owned secrets itself, so a profile asks again only when its
// secret is marked agent-owned or not-saved.

use crate::prompt::{self, Slot};
use std::collections::HashMap;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

pub const PATH: &str = "/org/freedesktop/NetworkManager/SecretAgent";
const ALLOW_INTERACTION: u32 = 0x1;
const REQUEST_NEW: u32 = 0x2;
pub const VPN_HINTS: u32 = 0x1;

pub type Settings = HashMap<String, HashMap<String, OwnedValue>>;

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.NetworkManager.SecretAgent")]
pub enum SecretError {
    #[zbus(error)]
    ZBus(zbus::Error),
    NoSecrets(String),
    UserCanceled(String),
}

/// One thing to ask for: the key NM wants it under, and how to ask.
#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    pub key: String,
    pub label: String,
    pub secret: bool,
}

fn plain<'a, 'v>(mut v: &'a Value<'v>) -> &'a Value<'v> {
    while let Value::Value(inner) = v {
        v = inner;
    }
    v
}

fn get_str(s: &Settings, group: &str, key: &str) -> Option<String> {
    match s.get(group)?.get(key).map(|v| plain(v)) {
        Some(Value::Str(x)) => Some(x.to_string()),
        _ => None,
    }
}

fn get_strs(s: &Settings, group: &str, key: &str) -> Vec<String> {
    match s.get(group).and_then(|g| g.get(key)).map(|v| plain(v)) {
        Some(Value::Array(a)) => a.iter().filter_map(|v| if let Value::Str(x) = plain(v) { Some(x.to_string()) } else { None }).collect(),
        _ => Vec::new(),
    }
}

/// The wifi network's name, for the prompt.
fn ssid(s: &Settings) -> Option<String> {
    match s.get("802-11-wireless")?.get("ssid").map(|v| plain(v)) {
        Some(Value::Array(a)) => {
            let b: Vec<u8> = a.iter().filter_map(|v| if let Value::U8(u) = plain(v) { Some(*u) } else { None }).collect();
            Some(String::from_utf8_lossy(&b).into_owned())
        }
        _ => None,
    }
}

/// What the connection is called, in words.
pub fn describe(s: &Settings) -> String {
    let id = get_str(s, "connection", "id").unwrap_or_else(|| "this connection".into());
    match get_str(s, "connection", "type").as_deref() {
        Some("802-11-wireless") => format!("Wi-Fi network {}", ssid(s).unwrap_or(id)),
        Some("vpn") => format!("VPN {id}"),
        Some("802-3-ethernet") => format!("wired connection {id}"),
        _ => id,
    }
}

/// What to ask for `setting`. None: a secret this agent doesn't know how to
/// ask for (NM then tries the next agent, or fails).
pub fn questions(s: &Settings, setting: &str, hints: &[String]) -> Option<Vec<Question>> {
    let q = |key: &str, label: &str, secret: bool| Question { key: key.into(), label: label.into(), secret };
    Some(match setting {
        "802-11-wireless-security" => match get_str(s, setting, "key-mgmt").as_deref() {
            Some("none") => vec![q("wep-key0", "WEP key", true)],
            Some("wpa-psk" | "sae") | None => vec![q("psk", "password", true)],
            _ => return None, // wpa-eap asks under "802-1x"
        },
        "802-1x" => {
            if get_strs(s, setting, "eap").iter().any(|e| e == "tls") {
                vec![q("private-key-password", "private key password", true)]
            } else {
                let mut v = Vec::new();
                if get_str(s, setting, "identity").is_none_or(|i| i.is_empty()) {
                    v.push(q("identity", "username", false));
                }
                v.push(q("password", "password", true));
                v
            }
        }
        "vpn" => {
            // With VPN_HINTS, NM names the secrets it wants; messages for the
            // person come through as x-vpn-message: hints.
            let names: Vec<&String> = hints.iter().filter(|h| !h.starts_with("x-vpn-message:")).collect();
            if names.is_empty() {
                vec![q("password", "password", true)]
            } else {
                names.iter().map(|n| q(n, &n.replace(['-', '_'], " "), true)).collect()
            }
        }
        "gsm" | "cdma" | "pppoe" => vec![q("password", "password", true)],
        _ => return None,
    })
}

/// NM's answer shape: {setting: {key: value}}; a VPN's go in vpn.secrets.
pub fn answer(setting: &str, given: Vec<(String, String)>) -> Settings {
    let mut inner: HashMap<String, OwnedValue> = HashMap::new();
    if setting == "vpn" {
        let secrets: HashMap<String, String> = given.into_iter().collect();
        if let Ok(v) = OwnedValue::try_from(Value::from(secrets)) {
            inner.insert("secrets".into(), v);
        }
    } else {
        for (k, v) in given {
            inner.insert(k, OwnedValue::from(zbus::zvariant::Str::from(v)));
        }
    }
    HashMap::from([(setting.to_string(), inner)])
}

#[derive(Default)]
pub struct Agent {
    slot: Slot,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.SecretAgent")]
impl Agent {
    async fn get_secrets(
        &self,
        connection: Settings,
        _connection_path: OwnedObjectPath,
        setting_name: String,
        hints: Vec<String>,
        flags: u32,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> Result<Settings, SecretError> {
        // Only NetworkManager may ask: the answer goes back to the caller.
        if !prompt::from_owner(conn, &hdr, "org.freedesktop.NetworkManager").await {
            return Err(SecretError::NoSecrets("not from NetworkManager".into()));
        }
        if flags & ALLOW_INTERACTION == 0 {
            return Err(SecretError::NoSecrets("no one to ask".into()));
        }
        let qs = questions(&connection, &setting_name, &hints)
            .ok_or_else(|| SecretError::NoSecrets(format!("lifeauth does not ask for {setting_name} secrets")))?;
        let what = describe(&connection);
        let note = hints.iter().find_map(|h| h.strip_prefix("x-vpn-message:")).map(str::to_string);
        let retry = flags & REQUEST_NEW != 0;
        let slot = self.slot.clone();
        let given = blocking::unblock(move || {
            let mut out = Vec::new();
            for q in qs {
                let mut mesg = format!("{what} needs a {}", q.label);
                if retry {
                    mesg = format!("That didn't work — {mesg}");
                }
                if let Some(n) = &note {
                    mesg = format!("{mesg} ({n})");
                }
                let a = prompt::ask(&q.label, &mesg, q.secret, &slot)?;
                out.push((q.key, a.to_string()));
            }
            Some(out)
        })
        .await
        .ok_or_else(|| SecretError::UserCanceled("dismissed".into()))?;
        Ok(answer(&setting_name, given))
    }

    fn cancel_get_secrets(&self, _connection_path: OwnedObjectPath, _setting_name: String) {
        prompt::cancel(&self.slot);
    }

    // NM keeps what it is allowed to keep; this agent stores nothing.
    fn save_secrets(&self, _connection: Settings, _connection_path: OwnedObjectPath) {}
    fn delete_secrets(&self, _connection: Settings, _connection_path: OwnedObjectPath) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(pairs: &[(&str, &str, Value<'static>)]) -> Settings {
        let mut m: Settings = HashMap::new();
        for (g, k, v) in pairs {
            m.entry(g.to_string()).or_default().insert(k.to_string(), OwnedValue::try_from(v.clone()).unwrap());
        }
        m
    }

    #[test]
    fn wifi_asks_for_the_right_key() {
        let c = s(&[
            ("connection", "id", Value::from("Cafe")),
            ("connection", "type", Value::from("802-11-wireless")),
            ("802-11-wireless", "ssid", Value::from(b"Cafe Free".to_vec())),
            ("802-11-wireless-security", "key-mgmt", Value::from("wpa-psk")),
        ]);
        assert_eq!(describe(&c), "Wi-Fi network Cafe Free");
        assert_eq!(questions(&c, "802-11-wireless-security", &[]).unwrap()[0].key, "psk");
        let wep = s(&[("802-11-wireless-security", "key-mgmt", Value::from("none"))]);
        assert_eq!(questions(&wep, "802-11-wireless-security", &[]).unwrap()[0].key, "wep-key0");
        let eap = s(&[("802-11-wireless-security", "key-mgmt", Value::from("wpa-eap"))]);
        assert!(questions(&eap, "802-11-wireless-security", &[]).is_none(), "asked under 802-1x instead");
    }

    #[test]
    fn enterprise_asks_identity_only_when_missing_and_tls_asks_the_key() {
        let peap = s(&[("802-1x", "eap", Value::from(vec!["peap"]))]);
        let q: Vec<String> = questions(&peap, "802-1x", &[]).unwrap().into_iter().map(|q| q.key).collect();
        assert_eq!(q, ["identity", "password"]);
        let known = s(&[("802-1x", "eap", Value::from(vec!["ttls"])), ("802-1x", "identity", Value::from("me@uni"))]);
        assert_eq!(questions(&known, "802-1x", &[]).unwrap().len(), 1);
        let tls = s(&[("802-1x", "eap", Value::from(vec!["tls"]))]);
        assert_eq!(questions(&tls, "802-1x", &[]).unwrap()[0].key, "private-key-password");
    }

    #[test]
    fn vpn_uses_the_hints_and_answers_inside_secrets() {
        let hints = vec!["x-vpn-message:Token from your app".to_string(), "password".into(), "challenge-response".into()];
        let q = questions(&HashMap::new(), "vpn", &hints).unwrap();
        assert_eq!(q.iter().map(|q| q.key.as_str()).collect::<Vec<_>>(), ["password", "challenge-response"]);
        let a = answer("vpn", vec![("password".into(), "hunter2".into())]);
        let secrets = a["vpn"]["secrets"].clone();
        let m: HashMap<String, String> = secrets.try_into().unwrap();
        assert_eq!(m["password"], "hunter2");
        let w = answer("802-11-wireless-security", vec![("psk".into(), "pw".into())]);
        assert_eq!(String::try_from(w["802-11-wireless-security"]["psk"].clone()).unwrap(), "pw");
    }

    #[test]
    fn unknown_settings_are_not_ours() {
        assert!(questions(&HashMap::new(), "wireguard", &[]).is_none());
    }
}
