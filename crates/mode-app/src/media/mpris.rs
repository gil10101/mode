//! Now playing on Linux (08 §6, 06 §12A): MPRIS 2 on the session bus over
//! gio's D-Bus, linked already through gtk (`gtk::gio`). Spotify first, else
//! the first player that is playing, else the first one listed.

use gtk::gio;
use gtk::glib::{self, ToVariant, Variant, VariantTy};

use super::{MediaError, NowPlaying, Player, Prop, Props, Snapshot, from_mpris, now_ms};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const SPOTIFY: &str = "org.mpris.MediaPlayer2.spotify";
const OBJECT: &str = "/org/mpris/MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";
const ROOT_IFACE: &str = "org.mpris.MediaPlayer2";
const TIMEOUT_MS: i32 = 500;

fn failed(e: glib::Error) -> MediaError {
    MediaError::Failed(e.to_string())
}

#[derive(Default)]
pub struct Mpris {
    bus: Option<gio::DBusConnection>,
    /// The bus name of the last snapshot; the commands go to it.
    current: Option<String>,
}

/// A `v` (or a bare value) as a [`Prop`].
fn prop(v: &Variant) -> Option<Prop> {
    let v = v.as_variant().unwrap_or_else(|| v.clone());
    if let Some(s) = v.str() {
        return Some(Prop::Str(s.to_string()));
    }
    if let Some(list) = v.get::<Vec<String>>() {
        return Some(Prop::Strs(list));
    }
    if let Some(n) = v.get::<i64>() {
        return Some(Prop::Int(n));
    }
    if let Some(n) = v.get::<u64>() {
        return Some(Prop::Int(n as i64));
    }
    if let Some(n) = v.get::<i32>() {
        return Some(Prop::Int(i64::from(n)));
    }
    None
}

/// An `a{sv}` as [`Props`].
fn props(dict: &Variant) -> Props {
    let mut out = Props::new();
    for i in 0..dict.n_children() {
        let entry = dict.child_value(i);
        if entry.n_children() < 2 {
            continue;
        }
        let key = entry.child_value(0);
        if let (Some(k), Some(v)) = (key.str(), prop(&entry.child_value(1))) {
            out.insert(k.to_string(), v);
        }
    }
    out
}

/// The value under `key` in an `a{sv}`, unboxed.
fn lookup(dict: &Variant, key: &str) -> Option<Variant> {
    (0..dict.n_children())
        .map(|i| dict.child_value(i))
        .filter(|e| e.n_children() >= 2)
        .find(|e| e.child_value(0).str() == Some(key))
        .map(|e| {
            let v = e.child_value(1);
            v.as_variant().unwrap_or(v)
        })
}

impl Mpris {
    fn bus(&mut self) -> Result<gio::DBusConnection, MediaError> {
        if let Some(b) = &self.bus {
            return Ok(b.clone());
        }
        let b =
            gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>).map_err(failed)?;
        self.bus = Some(b.clone());
        Ok(b)
    }

    fn call(
        &mut self,
        name: &str,
        path: &str,
        iface: &str,
        method: &str,
        args: Option<&Variant>,
        reply: Option<&VariantTy>,
    ) -> Result<Variant, MediaError> {
        self.bus()?
            .call_sync(
                Some(name),
                path,
                iface,
                method,
                args,
                reply,
                gio::DBusCallFlags::NONE,
                TIMEOUT_MS,
                None::<&gio::Cancellable>,
            )
            .map_err(failed)
    }

    /// Every MPRIS player on the bus, Spotify first.
    fn players(&mut self) -> Result<Vec<String>, MediaError> {
        let names = self.call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ListNames",
            None,
            VariantTy::new("(as)").ok(),
        )?;
        let mut list: Vec<String> = names
            .child_value(0)
            .get::<Vec<String>>()
            .unwrap_or_default()
            .into_iter()
            .filter(|n| n.starts_with(PREFIX))
            .collect();
        list.sort_by_key(|n| (n != SPOTIFY && !n.starts_with(&format!("{SPOTIFY}."))) as u8);
        Ok(list)
    }

    fn read(&mut self, name: &str) -> Option<NowPlaying> {
        let all = self
            .call(
                name,
                OBJECT,
                "org.freedesktop.DBus.Properties",
                "GetAll",
                Some(&(PLAYER_IFACE,).to_variant()),
                VariantTy::new("(a{sv})").ok(),
            )
            .ok()?;
        let p = props(&all.child_value(0));
        let status = match p.get("PlaybackStatus") {
            Some(Prop::Str(s)) => s.clone(),
            _ => return None,
        };
        let position = match p.get("Position") {
            Some(Prop::Int(n)) => *n,
            _ => 0,
        };
        let metadata = lookup(&all.child_value(0), "Metadata")
            .map(|m| props(&m))
            .unwrap_or_default();
        let app = name
            .strip_prefix(PREFIX)
            .and_then(|s| s.split('.').next())
            .map(|s| {
                let mut c = s.chars();
                c.next()
                    .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                    .unwrap_or_default()
            })
            .unwrap_or_else(|| "Media".into());
        from_mpris(&app, &status, &metadata, position, now_ms())
    }

    fn send(&mut self, iface: &str, method: &str) -> Result<(), MediaError> {
        let name = match self.current.clone() {
            Some(n) => n,
            None => self
                .players()?
                .into_iter()
                .next()
                .ok_or_else(|| MediaError::Failed("no player is running".into()))?,
        };
        self.call(&name, OBJECT, iface, method, None, None)
            .map(|_| ())
    }
}

impl Player for Mpris {
    fn snapshot(&mut self) -> Result<Snapshot, MediaError> {
        let mut paused: Option<(String, NowPlaying)> = None;
        let players = self.players()?;
        let any = !players.is_empty();
        for name in players {
            match self.read(&name) {
                Some(np) if np.playing => {
                    self.current = Some(name);
                    return Ok(Snapshot::Track(np));
                }
                Some(np) if paused.is_none() => paused = Some((name, np)),
                _ => {}
            }
        }
        match paused {
            Some((name, np)) => {
                self.current = Some(name);
                Ok(Snapshot::Track(np))
            }
            None => {
                self.current = None;
                // A player on the bus without a track is stopped, not gone.
                Ok(if any {
                    Snapshot::Stopped
                } else {
                    Snapshot::Gone
                })
            }
        }
    }

    fn play_pause(&mut self) -> Result<(), MediaError> {
        self.send(PLAYER_IFACE, "PlayPause")
    }

    fn next(&mut self) -> Result<(), MediaError> {
        self.send(PLAYER_IFACE, "Next")
    }

    fn previous(&mut self) -> Result<(), MediaError> {
        self.send(PLAYER_IFACE, "Previous")
    }

    fn open(&mut self) -> Result<(), MediaError> {
        self.send(ROOT_IFACE, "Raise")
    }
}
