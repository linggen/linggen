//! Reuse the session picture store, reference table, cache and existing UI.
use crate::engine::answer_refs::{self, table, Ref};
use crate::state_fs::sessions::{SessionMeta, SessionStore};
use anyhow::{Context, Result};
use base64::Engine;
use serde_json::{json, Value};

pub(super) fn keep(
    store: &SessionStore,
    session: &str,
    base64: &str,
    caption: &str,
) -> Result<Value> {
    let file = store
        .keep_images(session, &[base64.to_owned()])
        .pop()
        .context("Unsupported or oversized image")?;
    let mut id = String::new();
    table::number(&store.session_dir(session), |turn| {
        id = answer_refs::picture_id(turn, 1);
        let mut picture = Ref::picture(id.clone(), "", caption);
        picture.file = Some(file);
        vec![picture]
    })
    .context("Could not register the image")?;
    Ok(json!({"reference":id}))
}

pub(super) async fn display(
    store: &SessionStore,
    meta: &SessionMeta,
    source: &str,
    caption: &str,
) -> Result<Value> {
    if source.starts_with("https://") || source.starts_with("http://") {
        let mut id = String::new();
        table::number(&store.session_dir(&meta.id), |turn| {
            id = answer_refs::picture_id(turn, 1);
            vec![Ref::picture(id.clone(), source, caption)]
        })
        .context("Could not register the image")?;
        answer_refs::cache::show(store, &meta.id, &format!("[{id}]")).await;
        let shown = answer_refs::view::refs_for(store, &meta.id, [format!("[{id}]").as_str()]);
        anyhow::ensure!(
            shown.contains_key(&id),
            "The online image could not be loaded"
        );
        return Ok(json!({"reference":id}));
    }
    let path = if source.starts_with('~') || std::path::Path::new(source).is_absolute() {
        crate::util::resolve_path(std::path::Path::new(source))
    } else {
        std::path::Path::new(&meta.cwd).join(source)
    };
    anyhow::ensure!(
        tokio::fs::metadata(&path).await?.len() <= 12 * 1024 * 1024,
        "Image is too large"
    );
    let bytes = tokio::fs::read(path).await?;
    let bytes = crate::pictures::for_model::ready(&bytes)
        .context("Unsupported image")?
        .bytes;
    keep(
        store,
        &meta.id,
        &base64::engine::general_purpose::STANDARD.encode(bytes),
        caption,
    )
}
