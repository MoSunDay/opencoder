use super::*;

impl Journal {
    pub(super) fn load_legacy(&mut self) -> Result<()> {
        let root = self.layout.legacy_records_root();
        if !root.exists() {
            return Ok(());
        }
        if opencoder_core::platform::fs::is_link(&std::fs::symlink_metadata(&root)?) {
            bail!("legacy journal root cannot be a symlink");
        }
        for entry in std::fs::read_dir(root)? {
            let entry = entry?;
            if opencoder_core::platform::fs::is_link(&std::fs::symlink_metadata(entry.path())?) {
                bail!(
                    "journal records cannot be symlinks: {}",
                    entry.path().display()
                );
            }
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let (record, stamp) = io::read_stamped(&path, true)?;
            if path.file_stem().and_then(|value| value.to_str())
                != Some(record.assignment.index.id.as_str())
            {
                bail!("legacy journal file name does not match execution id");
            }
            self.insert_loaded(record, RecordLocation::Legacy, Some(stamp))?;
        }
        Ok(())
    }

    pub(super) fn load_current(&mut self) -> Result<()> {
        for kind in ALL_KINDS {
            let root = self.layout.kind_root(kind);
            if !root.exists() {
                continue;
            }
            if opencoder_core::platform::fs::is_link(&std::fs::symlink_metadata(&root)?) {
                bail!("execution kind root cannot be a symlink");
            }
            for entry in std::fs::read_dir(root)? {
                let entry = entry?;
                if entry.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                if opencoder_core::platform::fs::is_link(&std::fs::symlink_metadata(entry.path())?)
                {
                    bail!(
                        "execution directories cannot be symlinks: {}",
                        entry.path().display()
                    );
                }
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let id = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("non-UTF8 execution directory"))?;
                let path = self.layout.record_path(kind, &id)?;
                if !path.exists() {
                    continue;
                }
                let (record, stamp) = io::read_stamped(&path, false)?;
                validate_record(&record, Some(kind))?;
                if id != record.assignment.index.id {
                    bail!("execution directory name does not match execution id");
                }
                self.insert_loaded(record, RecordLocation::Current, Some(stamp))?;
            }
        }
        Ok(())
    }

    pub(super) fn insert_loaded(
        &mut self,
        record: Record,
        location: RecordLocation,
        stamp: Option<cache::Stamp>,
    ) -> Result<()> {
        let id = record.assignment.index.id.clone();
        if let Some(existing) = self.get_sync(&id)? {
            let existing_location = self.locations[&id];
            if !coexisting_records_are_valid(
                &self.layout,
                &existing,
                existing_location,
                &record,
                location,
            )? {
                bail!("conflicting legacy and current execution records for {id}");
            }
            if existing_location == RecordLocation::Current {
                return Ok(());
            }
        }
        self.locations.insert(id.clone(), location);
        self.update_report_candidate(&record);
        self.inventory.set(record.assignment.index.clone());
        let inspection =
            stamp.and_then(|stamp| Inspection::encode(&record).map(|bytes| (stamp, bytes)));
        self.install(record, location);
        if let Some((stamp, bytes)) = inspection {
            self.inspection_cache.lock().unwrap().insert(
                id,
                self.revision,
                stamp,
                std::sync::Arc::new(bytes),
            );
        }
        Ok(())
    }
}

fn coexisting_records_are_valid(
    layout: &DirectoryLayout,
    left: &Record,
    left_location: RecordLocation,
    right: &Record,
    right_location: RecordLocation,
) -> Result<bool> {
    if serde_json::to_value(left)? == serde_json::to_value(right)? {
        return Ok(true);
    }
    let (legacy, current) = match (left_location, right_location) {
        (RecordLocation::Legacy, RecordLocation::Current) => (left, right),
        (RecordLocation::Current, RecordLocation::Legacy) => (right, left),
        _ => return Ok(false),
    };
    Ok(same_execution(legacy, current)
        && crate::migration::receipt_matches_legacy(
            layout,
            &legacy.assignment.index.id,
            legacy.assignment.index.kind,
        )?)
}
