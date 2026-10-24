//! Pictures of games and tools: loaded and scaled down on a worker
//! thread, then kept for the session, so a redraw shows them at once.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use gtk::prelude::*;
use gtk::{gdk, gdk_pixbuf, gio, glib};
use protonhub_core::manifest::{Source, Tool};
use protonhub_core::releases::Client;

type Waiting = Vec<Box<dyn Fn(&gdk::Texture)>>;

enum State {
    Loading(Waiting),
    /// None when it couldn't be loaded; not tried again this session.
    Done(Option<gdk::Texture>),
}

thread_local! {
    static CACHE: RefCell<HashMap<String, State>> = RefCell::default();
}

/// Calls `set` with the picture at `source` (a local path or a URL),
/// `width` pixels wide, once it's loaded; never if it can't be.
pub fn load(client: &Arc<Client>, source: &str, width: i32, set: impl Fn(&gdk::Texture) + 'static) {
    let key = format!("{width}:{source}");
    let mut set = Some(Box::new(set) as Box<dyn Fn(&gdk::Texture)>);
    let (start, ready) = CACHE.with_borrow_mut(|cache| match cache.get_mut(&key) {
        Some(State::Done(texture)) => (false, texture.clone()),
        Some(State::Loading(waiting)) => {
            waiting.extend(set.take());
            (false, None)
        }
        None => {
            cache.insert(
                key.clone(),
                State::Loading(set.take().into_iter().collect()),
            );
            (true, None)
        }
    });
    if let (Some(set), Some(texture)) = (set, ready) {
        set(&texture);
    }
    if !start {
        return;
    }
    let (client, source) = (client.clone(), source.to_owned());
    let decoded = gio::spawn_blocking(move || {
        let path = if source.starts_with("https://") {
            client.fetch_file(&source).ok()?
        } else {
            PathBuf::from(source)
        };
        let pixbuf = gdk_pixbuf::Pixbuf::from_file_at_scale(&path, width, -1, true).ok()?;
        Some((
            pixbuf.read_pixel_bytes(),
            pixbuf.width(),
            pixbuf.height(),
            pixbuf.rowstride() as usize,
            pixbuf.has_alpha(),
        ))
    });
    glib::spawn_future_local(async move {
        let texture = decoded
            .await
            .ok()
            .flatten()
            .map(|(bytes, w, h, stride, alpha)| {
                let format = if alpha {
                    gdk::MemoryFormat::R8g8b8a8
                } else {
                    gdk::MemoryFormat::R8g8b8
                };
                gdk::MemoryTexture::new(w, h, format, &bytes, stride).upcast::<gdk::Texture>()
            });
        let waiting =
            CACHE.with_borrow_mut(
                |cache| match cache.insert(key, State::Done(texture.clone())) {
                    Some(State::Loading(waiting)) => waiting,
                    _ => Vec::new(),
                },
            );
        if let Some(t) = texture {
            for set in waiting {
                set(&t);
            }
        }
    });
}

/// Valve's picture, for its Protons: they're published as
/// ValveSoftware/Proton.
pub const VALVE: &str = "https://github.com/ValveSoftware.png?size=128";

/// A tool's picture: its GitHub owner's avatar, where it has one.
pub fn tool_source(tool: &Tool) -> Option<String> {
    match &tool.source {
        Source::Github(repo) => {
            let owner = repo.split('/').next()?;
            Some(format!("https://github.com/{owner}.png?size=128"))
        }
        _ => None,
    }
}

/// An avatar for a tool: its initials, then its picture once loaded.
pub fn tool_avatar(
    client: &Arc<Client>,
    tool: Option<&Tool>,
    name: &str,
    size: i32,
) -> adw::Avatar {
    avatar(client, name, tool.and_then(tool_source), size)
}

/// An avatar: `name`'s initials, then the picture at `source` once loaded.
pub fn avatar(client: &Arc<Client>, name: &str, source: Option<String>, size: i32) -> adw::Avatar {
    let avatar = adw::Avatar::new(size, Some(name), true);
    if let Some(source) = source {
        let weak = avatar.downgrade();
        load(client, &source, size * 2, move |t| {
            if let Some(a) = weak.upgrade() {
                a.set_custom_image(Some(t));
            }
        });
    }
    avatar
}
