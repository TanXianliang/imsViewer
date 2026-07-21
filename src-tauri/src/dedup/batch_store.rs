use super::batch_group::{auto_delete_candidates, build_groups, sort_groups_by_similarity};
use super::index::usable_index_path;
use super::types::{
    BatchCheckpoint, BatchGroup, BatchGroupMember, BatchGroupPage, BatchGroupSummary,
    BatchMemberPage, BatchMemberSource, BatchRunSummary, BatchScanRequest,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct BatchStore {
    root_dir: PathBuf,
    summary_path: PathBuf,
    groups_path: PathBuf,
    checkpoint_path: PathBuf,
}

impl BatchStore {
    pub fn create(
        app_data: &Path,
        run_id: &str,
        request: &BatchScanRequest,
        index_revision: &str,
    ) -> Result<Self, String> {
        validate_run_id(run_id)?;
        let store = Self::paths(app_data, run_id);
        if store.root_dir.exists() {
            return Err(format!("批量任务 ID 已存在: {}", run_id));
        }
        fs::create_dir_all(&store.root_dir)
            .map_err(|error| format!("创建批量任务目录失败: {}", error))?;
        let now = now_ms();
        let summary = BatchRunSummary {
            run_id: run_id.to_string(),
            root_paths: request.root_paths.clone(),
            mode: request.mode,
            status: "preparing".to_string(),
            index_revision: index_revision.to_string(),
            threshold: request.threshold,
            created_ms: now,
            updated_ms: now,
            processed_pairs: 0,
            total_pairs: 0,
            candidate_pairs: 0,
            accepted_pairs: 0,
            group_count: 0,
            resumable: false,
            error_message: None,
        };
        write_json_atomic(&store.summary_path, &summary)?;
        Ok(store)
    }

    pub fn open(app_data: &Path, run_id: &str) -> Result<Self, String> {
        validate_run_id(run_id)?;
        let store = Self::paths(app_data, run_id);
        if !store.summary_path.is_file() {
            return Err(format!("批量任务不存在: {}", run_id));
        }
        Ok(store)
    }

    pub fn list(app_data: &Path) -> Result<Vec<BatchRunSummary>, String> {
        let runs_dir = app_data.join("dedup").join("batch-runs");
        if !runs_dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut runs = Vec::new();
        for entry in
            fs::read_dir(&runs_dir).map_err(|error| format!("读取批量任务目录失败: {}", error))?
        {
            let entry = match entry {
                Ok(value) if value.path().is_dir() => value,
                _ => continue,
            };
            let summary_path = entry.path().join("summary.json");
            if let Ok(summary) = read_json::<BatchRunSummary>(&summary_path) {
                runs.push(summary);
            }
        }
        runs.sort_by(|left, right| right.created_ms.cmp(&left.created_ms));
        Ok(runs)
    }

    pub fn request(&self) -> Result<BatchScanRequest, String> {
        let summary = self.summary()?;
        Ok(BatchScanRequest {
            root_paths: summary.root_paths,
            mode: summary.mode,
            threshold: summary.threshold,
        })
    }

    pub fn summary(&self) -> Result<BatchRunSummary, String> {
        read_json(&self.summary_path)
    }

    pub fn set_total(&self, total_pairs: u64) -> Result<(), String> {
        self.update_summary(|summary| {
            summary.total_pairs = total_pairs;
            summary.status = "running".to_string();
            summary.error_message = None;
        })
    }

    pub fn update_progress(
        &self,
        processed_pairs: u64,
        candidate_pairs: u64,
        accepted_pairs: u64,
    ) -> Result<(), String> {
        self.update_summary(|summary| {
            summary.processed_pairs = processed_pairs;
            summary.candidate_pairs = candidate_pairs;
            summary.accepted_pairs = accepted_pairs;
        })
    }

    pub fn set_status(
        &self,
        status: &str,
        error_message: Option<&str>,
        resumable: bool,
    ) -> Result<(), String> {
        self.update_summary(|summary| {
            summary.status = status.to_string();
            summary.error_message = error_message.map(str::to_string);
            summary.resumable = resumable;
        })
    }

    pub fn replace_groups(&self, groups: &[BatchGroup]) -> Result<(), String> {
        write_json_atomic(&self.groups_path, groups)?;
        self.update_summary(|summary| {
            summary.group_count = groups.len() as u64;
            summary.status = "completed".to_string();
            summary.resumable = false;
            summary.error_message = None;
        })
    }

    pub fn get_groups(&self, page: u64, page_size: u64) -> Result<BatchGroupPage, String> {
        let mut groups = self.groups()?;
        sort_groups_by_similarity(&mut groups);
        let page_size = page_size.clamp(1, 200);
        let start = page.saturating_mul(page_size) as usize;
        let end = start.saturating_add(page_size as usize).min(groups.len());
        let items = if start >= groups.len() {
            Vec::new()
        } else {
            groups[start..end]
                .iter()
                .map(|group| BatchGroupSummary {
                    group_id: group.group_id,
                    member_count: group.member_count,
                    total_bytes: group.total_bytes,
                    reclaimable_bytes: group.reclaimable_bytes,
                    recommended_item_id: group.recommended_item_id,
                    min_similarity: group.min_similarity,
                    max_similarity: group.max_similarity,
                })
                .collect()
        };
        Ok(BatchGroupPage {
            page,
            page_size,
            total: groups.len() as u64,
            items,
        })
    }

    pub fn get_group_members(
        &self,
        group_id: i64,
        page: u64,
        page_size: u64,
    ) -> Result<BatchMemberPage, String> {
        let groups = self.groups()?;
        let group = groups
            .into_iter()
            .find(|group| group.group_id == group_id)
            .ok_or_else(|| format!("重复组不存在: {}", group_id))?;
        let page_size = page_size.clamp(1, 200);
        let start = page.saturating_mul(page_size) as usize;
        let end = start
            .saturating_add(page_size as usize)
            .min(group.members.len());
        let items = if start >= group.members.len() {
            Vec::new()
        } else {
            group.members[start..end]
                .iter()
                .cloned()
                .map(|mut member| {
                    member.filepath = usable_index_path(&member.filepath)
                        .to_string_lossy()
                        .to_string();
                    member
                })
                .collect()
        };
        Ok(BatchMemberPage {
            group_id,
            page,
            page_size,
            total: group.members.len() as u64,
            items,
        })
    }

    pub fn auto_delete_candidates(&self, threshold: f32) -> Result<Vec<BatchGroupMember>, String> {
        Ok(auto_delete_candidates(&self.groups()?, threshold))
    }

    pub fn remove_members(&self, removed: &HashSet<i64>) -> Result<u64, String> {
        if removed.is_empty() {
            return Ok(self.summary()?.group_count);
        }
        let groups = self.groups()?;
        let members = groups
            .iter()
            .flat_map(|group| group.members.iter())
            .filter(|member| !removed.contains(&member.batch_item_id))
            .map(|member| BatchMemberSource {
                batch_item_id: member.batch_item_id,
                item_id: member.item_id,
                index_root_path: member.index_root_path.clone(),
                filepath: usable_index_path(&member.filepath)
                    .to_string_lossy()
                    .to_string(),
                filename: member.filename.clone(),
                kind: member.kind.clone(),
                image_format: member.image_format.clone(),
                width: member.width,
                height: member.height,
                file_size: member.file_size,
                modified_ms: member.modified_ms,
            })
            .collect::<Vec<_>>();
        let member_ids = members
            .iter()
            .map(|member| member.batch_item_id)
            .collect::<HashSet<_>>();
        let edges = groups
            .iter()
            .flat_map(|group| group.edges.iter())
            .filter(|edge| {
                member_ids.contains(&edge.left_item_id) && member_ids.contains(&edge.right_item_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        let rebuilt = build_groups(&members, &edges);
        write_json_atomic(&self.groups_path, &rebuilt)?;
        self.update_summary(|summary| summary.group_count = rebuilt.len() as u64)?;
        Ok(rebuilt.len() as u64)
    }

    pub fn validate_revision(&self, revision: &str) -> Result<(), String> {
        let summary = self.summary()?;
        if summary.index_revision == revision {
            Ok(())
        } else {
            Err("图库索引版本已经变化，无法继续旧任务".to_string())
        }
    }

    pub fn mark_stale_if_needed(&self, revision: &str) -> Result<bool, String> {
        if self.summary()?.index_revision == revision {
            return Ok(false);
        }
        self.set_status("stale", Some("图库索引已更新，结果可能已过期"), false)?;
        Ok(true)
    }

    pub fn save_checkpoint(&self, checkpoint: &BatchCheckpoint) -> Result<(), String> {
        write_json_atomic(&self.checkpoint_path, checkpoint)
    }

    pub fn load_checkpoint(&self) -> Result<Option<BatchCheckpoint>, String> {
        if !self.checkpoint_path.is_file() {
            return Ok(None);
        }
        read_json(&self.checkpoint_path).map(Some)
    }

    pub fn remove_checkpoint(&self) -> Result<(), String> {
        remove_if_exists(&self.checkpoint_path)
    }

    pub fn delete(self) -> Result<u64, String> {
        let released_bytes = directory_size(&self.root_dir);
        fs::remove_dir_all(&self.root_dir)
            .map_err(|error| format!("删除批量任务失败: {}", error))?;
        Ok(released_bytes)
    }

    fn paths(app_data: &Path, run_id: &str) -> Self {
        let root_dir = app_data.join("dedup").join("batch-runs").join(run_id);
        Self {
            summary_path: root_dir.join("summary.json"),
            groups_path: root_dir.join("groups.json"),
            checkpoint_path: root_dir.join("checkpoint.json"),
            root_dir,
        }
    }

    fn groups(&self) -> Result<Vec<BatchGroup>, String> {
        if !self.groups_path.is_file() {
            return Ok(Vec::new());
        }
        read_json(&self.groups_path)
    }

    fn update_summary(&self, update: impl FnOnce(&mut BatchRunSummary)) -> Result<(), String> {
        let mut summary = self.summary()?;
        update(&mut summary);
        summary.updated_ms = now_ms();
        write_json_atomic(&self.summary_path, &summary)
    }
}

fn validate_run_id(run_id: &str) -> Result<(), String> {
    if run_id.is_empty()
        || run_id.len() > 64
        || !run_id
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || value == b'-')
    {
        return Err("批量任务 ID 格式无效".to_string());
    }
    Ok(())
}

fn write_json_atomic(path: &Path, value: &(impl Serialize + ?Sized)) -> Result<(), String> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| format!("序列化批量任务失败: {}", error))?;
    let temporary = path.with_extension("tmp");
    let previous = path.with_extension("prev");
    remove_if_exists(&temporary)?;
    remove_if_exists(&previous)?;
    let mut file =
        File::create(&temporary).map_err(|error| format!("创建批量任务临时文件失败: {}", error))?;
    file.write_all(&bytes)
        .map_err(|error| format!("写入批量任务临时文件失败: {}", error))?;
    file.sync_all()
        .map_err(|error| format!("同步批量任务临时文件失败: {}", error))?;
    drop(file);
    if path.exists() {
        fs::rename(path, &previous).map_err(|error| format!("备份批量任务文件失败: {}", error))?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if previous.exists() {
            let _ = fs::rename(&previous, path);
        }
        return Err(format!("安装批量任务文件失败: {}", error));
    }
    remove_if_exists(&previous)?;
    Ok(())
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let bytes = fs::read(path)
        .or_else(|_| fs::read(path.with_extension("prev")))
        .map_err(|error| format!("读取批量任务文件失败 {}: {}", path.display(), error))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("解析批量任务文件失败 {}: {}", path.display(), error))
}

fn remove_if_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_file(path)
            .map_err(|error| format!("删除文件失败 {}: {}", path.display(), error))?;
    }
    Ok(())
}

fn directory_size(root: &Path) -> u64 {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dedup::types::{AcceptedEdge, BatchMemberSource};

    #[test]
    fn run_store_rejects_escaping_id_and_round_trips_checkpoint() {
        let app_data = tempfile::tempdir().unwrap();
        let request = BatchScanRequest {
            root_paths: vec!["C:/gallery".to_string()],
            mode: crate::dedup::types::BatchScanMode::Complete,
            threshold: 0.85,
        };
        assert!(BatchStore::create(app_data.path(), "../bad", &request, "rev").is_err());
        let store = BatchStore::create(app_data.path(), "run-1", &request, "rev").unwrap();
        let checkpoint = BatchCheckpoint {
            next_left_block: 2,
            processed_pairs: 10,
            candidate_pairs: 3,
            parent: vec![0, 0],
            rank: vec![1, 0],
            accepted_edges: vec![AcceptedEdge::new(1, 2, 0.9, "sscd")],
        };
        store.save_checkpoint(&checkpoint).unwrap();
        assert_eq!(store.load_checkpoint().unwrap(), Some(checkpoint));
        assert!(store.validate_revision("rev").is_ok());
        assert!(store.validate_revision("changed").is_err());
    }

    #[test]
    fn removing_deleted_members_rebuilds_visible_groups() {
        let app_data = tempfile::tempdir().unwrap();
        let request = BatchScanRequest {
            root_paths: vec!["C:/gallery".to_string()],
            mode: crate::dedup::types::BatchScanMode::Quick,
            threshold: 0.85,
        };
        let store = BatchStore::create(app_data.path(), "run-2", &request, "rev").unwrap();
        let members = (1..=3)
            .map(|id| BatchMemberSource {
                batch_item_id: id,
                item_id: id,
                index_root_path: "C:/gallery".to_string(),
                filepath: format!("C:/gallery/{}.png", id),
                filename: format!("{}.png", id),
                kind: "standard".to_string(),
                image_format: "PNG".to_string(),
                width: id as u32,
                height: 1,
                file_size: id as u64,
                modified_ms: 1,
            })
            .collect::<Vec<_>>();
        store
            .replace_groups(&build_groups(
                &members,
                &[
                    AcceptedEdge::new(1, 2, 0.99, "sscd"),
                    AcceptedEdge::new(2, 3, 0.98, "sscd"),
                ],
            ))
            .unwrap();

        assert_eq!(store.remove_members(&HashSet::from([1])).unwrap(), 1);
        let groups = store.get_groups(0, 10).unwrap();
        assert_eq!(groups.total, 1);
        let remaining = store
            .get_group_members(groups.items[0].group_id, 0, 10)
            .unwrap();
        assert_eq!(
            remaining
                .items
                .iter()
                .map(|member| member.batch_item_id)
                .collect::<Vec<_>>(),
            vec![3, 2]
        );

        assert_eq!(store.remove_members(&HashSet::from([2])).unwrap(), 0);
        assert_eq!(store.get_groups(0, 10).unwrap().total, 0);
    }
}
