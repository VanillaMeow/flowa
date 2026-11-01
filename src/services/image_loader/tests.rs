use std::{fs::File, path::Path, sync::Arc, time::Duration};

use super::*;

const IMAGE_BUDGET: u64 = 1024 * 1024;

fn save_image(path: &Path) -> Result<()> {
    image::RgbaImage::new(32, 16).save(path)?;

    Ok(())
}

#[test]
fn queue_keeps_latest_selection_and_prioritizes_hover() -> Result<()> {
    let mut queue = Queue::default();
    queue.receive(Message::Neighbors(vec![
        "next.png".into(),
        "previous.png".into(),
    ]));
    queue.receive(Message::Hover("hover.png".into()));
    queue.receive(Message::Hover("hover.png".into()));
    assert_eq!(queue.preloads.len(), 3);
    assert_eq!(
        queue.next().context("Missing hover")?.path,
        Path::new("hover.png")
    );

    for generation in 1..=3 {
        queue.receive(Message::Select(LoadRequest {
            generation,
            path: "selected.png".into(),
            budget: IMAGE_BUDGET,
        }));
    }

    let selected = queue.next().context("Missing selection")?;
    assert_eq!(selected.generation, Some(3));
    assert!(queue.next().is_none());

    Ok(())
}

#[test]
fn click_during_preload_adopts_the_same_pixels() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("image.png");
    save_image(&path)?;
    let mut queue = Queue::default();
    queue.receive(Message::Budget(IMAGE_BUDGET));
    queue.receive(Message::Hover(path.clone()));
    let job = queue.next().context("Missing preload")?;
    let revision = queue.revision;
    let stamp = FileStamp::read(&path);
    let decoded = queue.load(&job, stamp).context("Preload skipped")??;
    let pixels = Arc::clone(&decoded.frames[0].pixels);

    queue.receive(Message::Select(LoadRequest {
        generation: 2,
        path,
        budget: IMAGE_BUDGET,
    }));
    let completed = queue.finish(job, stamp, revision, Ok(decoded), 2);
    let decoded = completed.context("Preload was not promoted")?.result?;
    assert!(Arc::ptr_eq(&pixels, &decoded.frames[0].pixels));
    assert!(queue.next().is_none());

    Ok(())
}

#[test]
fn cache_reuses_pixels_and_rejects_modified_files() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("image.png");
    save_image(&path)?;
    let stamp = FileStamp::read(&path).context("Missing file timestamp")?;
    let mut cache = ImageCache::default();
    cache.set_budget(IMAGE_BUDGET);
    let decoded = decode(&path, IMAGE_BUDGET)?;
    let pixels = Arc::clone(&decoded.frames[0].pixels);
    cache.insert(path.clone(), stamp, decoded);

    let cached = cache
        .take(&path, Some(stamp))
        .context("Missing cached image")?;
    assert!(Arc::ptr_eq(&pixels, &cached.frames[0].pixels));
    cache.insert(path.clone(), stamp, cached);

    let modified = path.metadata()?.modified()? + Duration::from_secs(2);
    File::options()
        .write(true)
        .open(&path)?
        .set_modified(modified)?;
    assert!(cache.take(&path, FileStamp::read(&path)).is_none());

    Ok(())
}

#[test]
fn speculation_reserves_working_memory_and_skips_oversized_images() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let first = directory.path().join("first.png");
    let second = directory.path().join("second.png");
    save_image(&first)?;
    save_image(&second)?;
    let reserved =
        preload_bytes(&second, IMAGE_BUDGET)?.context("Image was skipped")?;
    let mut queue = Queue::default();
    queue.receive(Message::Budget(reserved * 16));
    let decoded = decode(&first, IMAGE_BUDGET)?;
    assert!(reserved > decoded.memory_bytes());
    queue.cache.insert(
        first.clone(),
        FileStamp::read(&first).context("Missing timestamp")?,
        decoded,
    );

    queue.receive(Message::Hover(second.clone()));
    let job = queue.next().context("Missing preload")?;
    assert!(
        queue
            .load(&job, FileStamp::read(&second))
            .context("Preload skipped")?
            .is_ok()
    );
    assert!(!queue.cache.contains(&first, FileStamp::read(&first)));

    queue.receive(Message::Budget((reserved - 1) * 16));
    queue.receive(Message::Hover(second.clone()));
    let job = queue.next().context("Missing preload")?;
    assert!(queue.load(&job, FileStamp::read(&second)).is_none());

    Ok(())
}

#[test]
fn invalidation_discards_in_flight_preloads_and_preserves_the_click() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("image.png");
    save_image(&path)?;
    let mut queue = Queue::default();
    queue.receive(Message::Budget(IMAGE_BUDGET));
    queue.receive(Message::Hover(path.clone()));
    let job = queue.next().context("Missing preload")?;
    let revision = queue.revision;
    let stamp = FileStamp::read(&path);
    let result = queue.load(&job, stamp).context("Preload skipped")?;

    queue.receive(Message::Invalidate);
    queue.receive(Message::Select(LoadRequest {
        generation: 2,
        path: path.clone(),
        budget: IMAGE_BUDGET,
    }));
    assert!(queue.finish(job, stamp, revision, result, 2).is_none());
    assert!(!queue.cache.contains(&path, stamp));
    assert_eq!(queue.next().context("Click was lost")?.generation, Some(2));

    Ok(())
}
