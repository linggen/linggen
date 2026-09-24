use super::*;

fn surface_at(base: &std::path::Path, cfg: serde_json::Value) -> Surface {
    Surface {
        skill: "s".into(),
        cfg: serde_json::from_value(cfg).unwrap(),
        base: base.to_path_buf(),
    }
}

#[test]
fn items_carry_companions_with_sizes() {
    let d = tempfile::tempdir().unwrap();
    let base = d.path();
    std::fs::create_dir(base.join(".extra")).unwrap();
    std::fs::write(base.join("b.m4a"), b"bb").unwrap();
    std::fs::write(base.join("a.MP3"), b"aaaa").unwrap();
    std::fs::write(base.join("a.lrc"), b"lyr").unwrap();
    std::fs::write(base.join("notes.txt"), b"x").unwrap();
    std::fs::write(base.join("b.m4a.part"), b"partial").unwrap();
    std::fs::write(base.join(".extra/b.k.json"), b"12345").unwrap();
    let s = surface_at(
        base,
        serde_json::json!({
            "dir": base.to_string_lossy(),
            "items": ["mp3", "m4a"],
            "subdirs": { "extra": ".extra" },
            "companions": [
                { "name": "text", "exts": ["txt", "lrc"] },
                { "name": "side", "exts": ["json"], "subdir": "extra", "suffix": ".k" }
            ]
        }),
    );
    let items = build_items(&s, listing::list);
    let out = serde_json::to_string(&items).unwrap();
    assert_eq!(
        out,
        r#"[{"name":"a.MP3","size":4,"text_size":3,"text":"a.lrc","side":null},{"name":"b.m4a","size":2,"text":null,"side_size":5,"side":"b.k.json"}]"#
    );
}

#[test]
fn scratch_files_are_not_changes() {
    use std::path::Path;
    for p in [
        "/x/song.m4a.part",
        "/x/song.f140.m4a.part-Frag3",
        "/x/.DS_Store",
        "/x/song.ytdl",
        "/x/state.json.tmp",
    ] {
        assert!(is_scratch(Path::new(p)), "{p}");
    }
    for p in ["/x/song.m4a", "/x/.extra/song.lrc", "/x/partial.mp3"] {
        assert!(!is_scratch(Path::new(p)), "{p}");
    }
}

#[tokio::test]
async fn file_streams_whole_and_in_order() {
    use futures_util::StreamExt;
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("big.bin");
    let data: Vec<u8> = (0..(STREAM_CHUNK * 3 + 17)).map(|i| i as u8).collect();
    std::fs::write(&path, &data).unwrap();
    let (file, len) = open_file(&path).await.unwrap();
    assert_eq!(len, data.len() as u64);
    let mut got = Vec::new();
    let mut chunks = std::pin::pin!(file_chunks(file));
    while let Some(c) = chunks.next().await {
        got.extend_from_slice(&c.unwrap());
    }
    assert_eq!(got, data);
    assert!(open_file(d.path()).await.is_none(), "a dir is not a file");
}
