use super::index::normalize_path;
use super::types::{AcceptedEdge, BatchGroup, BatchGroupMember, BatchMemberSource};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ConnectivityState {
    index_by_id: HashMap<i64, usize>,
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl ConnectivityState {
    pub fn new(item_ids: Vec<i64>) -> Self {
        let index_by_id = item_ids
            .iter()
            .enumerate()
            .map(|(index, item_id)| (*item_id, index))
            .collect();
        let parent = (0..item_ids.len()).collect();
        let rank = vec![0; item_ids.len()];
        Self {
            index_by_id,
            parent,
            rank,
        }
    }

    pub fn from_parts(
        item_ids: Vec<i64>,
        parent: Vec<usize>,
        rank: Vec<u8>,
    ) -> Result<Self, String> {
        if parent.len() != item_ids.len() || rank.len() != item_ids.len() {
            return Err("批量检查点并查集长度不匹配".to_string());
        }
        if parent.iter().any(|index| *index >= item_ids.len()) {
            return Err("批量检查点并查集索引越界".to_string());
        }
        let index_by_id = item_ids
            .iter()
            .enumerate()
            .map(|(index, item_id)| (*item_id, index))
            .collect::<HashMap<_, _>>();
        if index_by_id.len() != item_ids.len() {
            return Err("批量检查点包含重复成员 ID".to_string());
        }
        Ok(Self {
            index_by_id,
            parent,
            rank,
        })
    }

    pub fn connect(&mut self, left_item_id: i64, right_item_id: i64) -> Result<bool, String> {
        let left = *self
            .index_by_id
            .get(&left_item_id)
            .ok_or_else(|| format!("未知批量成员 ID: {}", left_item_id))?;
        let right = *self
            .index_by_id
            .get(&right_item_id)
            .ok_or_else(|| format!("未知批量成员 ID: {}", right_item_id))?;
        let left_root = self.find(left);
        let right_root = self.find(right);
        if left_root == right_root {
            return Ok(false);
        }
        match self.rank[left_root].cmp(&self.rank[right_root]) {
            std::cmp::Ordering::Less => self.parent[left_root] = right_root,
            std::cmp::Ordering::Greater => self.parent[right_root] = left_root,
            std::cmp::Ordering::Equal => {
                self.parent[right_root] = left_root;
                self.rank[left_root] = self.rank[left_root].saturating_add(1);
            }
        }
        Ok(true)
    }

    pub fn checkpoint_parts(&self) -> (Vec<usize>, Vec<u8>) {
        (self.parent.clone(), self.rank.clone())
    }

    fn find(&mut self, index: usize) -> usize {
        if self.parent[index] != index {
            self.parent[index] = self.find(self.parent[index]);
        }
        self.parent[index]
    }
}

pub fn build_groups(members: &[BatchMemberSource], edges: &[AcceptedEdge]) -> Vec<BatchGroup> {
    let mut connectivity =
        ConnectivityState::new(members.iter().map(|member| member.batch_item_id).collect());
    for edge in edges {
        let _ = connectivity.connect(edge.left_item_id, edge.right_item_id);
    }

    let mut components = BTreeMap::<usize, Vec<&BatchMemberSource>>::new();
    for (index, member) in members.iter().enumerate() {
        let root = connectivity.find(index);
        components.entry(root).or_default().push(member);
    }

    let mut groups = components
        .into_values()
        .filter(|component| component.len() >= 2)
        .map(|component| build_group(component, edges))
        .collect::<Vec<_>>();
    sort_groups_by_similarity(&mut groups);
    groups
}

pub fn sort_groups_by_similarity(groups: &mut [BatchGroup]) {
    groups.sort_by(|left, right| {
        right
            .min_similarity
            .total_cmp(&left.min_similarity)
            .then_with(|| right.max_similarity.total_cmp(&left.max_similarity))
            .then_with(|| right.reclaimable_bytes.cmp(&left.reclaimable_bytes))
            .then_with(|| left.group_id.cmp(&right.group_id))
    });
}

pub fn auto_delete_candidates(groups: &[BatchGroup], threshold: f32) -> Vec<BatchGroupMember> {
    let mut candidates = Vec::new();
    for group in groups {
        let member_ids = group
            .members
            .iter()
            .map(|member| member.batch_item_id)
            .collect::<std::collections::HashSet<_>>();
        let mut connectivity = ConnectivityState::new(
            group
                .members
                .iter()
                .map(|member| member.batch_item_id)
                .collect(),
        );
        for edge in &group.edges {
            if edge.similarity > threshold
                && member_ids.contains(&edge.left_item_id)
                && member_ids.contains(&edge.right_item_id)
            {
                let _ = connectivity.connect(edge.left_item_id, edge.right_item_id);
            }
        }

        let mut components = BTreeMap::<usize, Vec<&BatchGroupMember>>::new();
        for (index, member) in group.members.iter().enumerate() {
            let root = connectivity.find(index);
            components.entry(root).or_default().push(member);
        }
        for component in components.into_values().filter(|items| items.len() >= 2) {
            let keeper = component
                .iter()
                .copied()
                .find(|member| member.recommended)
                .or_else(|| {
                    component.iter().copied().max_by(|left, right| {
                        let left_pixels = u64::from(left.width) * u64::from(left.height);
                        let right_pixels = u64::from(right.width) * u64::from(right.height);
                        left_pixels
                            .cmp(&right_pixels)
                            .then_with(|| left.file_size.cmp(&right.file_size))
                            .then_with(|| {
                                normalize_path(Path::new(&right.filepath))
                                    .cmp(&normalize_path(Path::new(&left.filepath)))
                            })
                    })
                })
                .expect("duplicate component has members");
            candidates.extend(
                component
                    .into_iter()
                    .filter(|member| member.batch_item_id != keeper.batch_item_id)
                    .cloned(),
            );
        }
    }
    candidates.sort_by(|left, right| {
        normalize_path(Path::new(&left.filepath)).cmp(&normalize_path(Path::new(&right.filepath)))
    });
    candidates
}

fn build_group(component: Vec<&BatchMemberSource>, edges: &[AcceptedEdge]) -> BatchGroup {
    let member_ids = component
        .iter()
        .map(|member| member.batch_item_id)
        .collect::<std::collections::HashSet<_>>();
    let mut group_edges = edges
        .iter()
        .filter(|edge| {
            member_ids.contains(&edge.left_item_id) && member_ids.contains(&edge.right_item_id)
        })
        .cloned()
        .collect::<Vec<_>>();
    group_edges.sort_by(|left, right| {
        left.left_item_id
            .cmp(&right.left_item_id)
            .then_with(|| left.right_item_id.cmp(&right.right_item_id))
    });

    let recommended = component
        .iter()
        .copied()
        .max_by(|left, right| {
            let left_pixels = u64::from(left.width) * u64::from(left.height);
            let right_pixels = u64::from(right.width) * u64::from(right.height);
            left_pixels
                .cmp(&right_pixels)
                .then_with(|| left.file_size.cmp(&right.file_size))
                .then_with(|| {
                    normalize_path(Path::new(&right.filepath))
                        .cmp(&normalize_path(Path::new(&left.filepath)))
                })
        })
        .expect("duplicate group has members");
    let total_bytes = component.iter().map(|member| member.file_size).sum::<u64>();
    let group_id = component
        .iter()
        .map(|member| member.batch_item_id)
        .min()
        .unwrap_or(recommended.batch_item_id);
    let min_similarity = group_edges
        .iter()
        .map(|edge| edge.similarity)
        .reduce(f32::min)
        .unwrap_or(1.0);
    let max_similarity = group_edges
        .iter()
        .map(|edge| edge.similarity)
        .reduce(f32::max)
        .unwrap_or(1.0);

    let mut group_members = component
        .into_iter()
        .map(|member| {
            let link = group_edges
                .iter()
                .filter(|edge| {
                    edge.left_item_id == member.batch_item_id
                        || edge.right_item_id == member.batch_item_id
                })
                .max_by(|left, right| left.similarity.total_cmp(&right.similarity));
            BatchGroupMember {
                batch_item_id: member.batch_item_id,
                item_id: member.item_id,
                index_root_path: member.index_root_path.clone(),
                filepath: member.filepath.clone(),
                filename: member.filename.clone(),
                kind: member.kind.clone(),
                image_format: member.image_format.clone(),
                width: member.width,
                height: member.height,
                file_size: member.file_size,
                modified_ms: member.modified_ms,
                linked_item_id: link.map(|edge| {
                    if edge.left_item_id == member.batch_item_id {
                        edge.right_item_id
                    } else {
                        edge.left_item_id
                    }
                }),
                similarity: link.map(|edge| edge.similarity),
                reason: link.map(|edge| edge.reason.clone()),
                recommended: member.batch_item_id == recommended.batch_item_id,
            }
        })
        .collect::<Vec<_>>();
    group_members.sort_by(|left, right| {
        right.recommended.cmp(&left.recommended).then_with(|| {
            normalize_path(Path::new(&left.filepath))
                .cmp(&normalize_path(Path::new(&right.filepath)))
        })
    });

    BatchGroup {
        group_id,
        member_count: group_members.len() as u64,
        total_bytes,
        reclaimable_bytes: total_bytes.saturating_sub(recommended.file_size),
        recommended_item_id: recommended.batch_item_id,
        min_similarity,
        max_similarity,
        members: group_members,
        edges: group_edges,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: i64, width: u32, bytes: u64, path: &str) -> BatchMemberSource {
        BatchMemberSource {
            batch_item_id: id,
            item_id: id,
            index_root_path: "C:/gallery".to_string(),
            filepath: path.to_string(),
            filename: path.to_string(),
            kind: "standard".to_string(),
            image_format: "PNG".to_string(),
            width,
            height: 1,
            file_size: bytes,
            modified_ms: 1,
        }
    }

    #[test]
    fn transitive_edges_form_one_group() {
        let members = vec![
            member(1, 100, 10, "a.png"),
            member(2, 200, 20, "b.png"),
            member(3, 150, 30, "c.png"),
            member(4, 50, 5, "unrelated.png"),
        ];
        let edges = vec![
            AcceptedEdge::new(1, 2, 0.92, "sscd"),
            AcceptedEdge::new(2, 3, 0.89, "sscd"),
        ];
        let groups = build_groups(&members, &edges);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].recommended_item_id, 2);
        assert_eq!(groups[0].members.len(), 3);
    }

    #[test]
    fn recommendation_ties_break_by_path() {
        let members = vec![member(7, 100, 20, "z.png"), member(9, 100, 20, "a.png")];
        let groups = build_groups(&members, &[AcceptedEdge::new(7, 9, 1.0, "sha256")]);
        assert_eq!(groups[0].recommended_item_id, 9);
    }

    #[test]
    fn auto_delete_keeps_one_member_per_above_threshold_component() {
        let members = vec![
            member(1, 100, 10, "a.png"),
            member(2, 200, 20, "b.png"),
            member(3, 300, 30, "c.png"),
        ];
        let groups = build_groups(
            &members,
            &[
                AcceptedEdge::new(1, 2, 0.995, "sscd"),
                AcceptedEdge::new(2, 3, 0.95, "sscd"),
            ],
        );
        let candidates = auto_delete_candidates(&groups, 0.99);
        assert_eq!(
            candidates
                .iter()
                .map(|member| member.batch_item_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert!(auto_delete_candidates(&groups, 0.995).is_empty());
    }

    #[test]
    fn groups_are_sorted_by_minimum_similarity_descending() {
        let members = vec![
            member(1, 100, 10, "a.png"),
            member(2, 100, 10, "b.png"),
            member(3, 100, 10, "c.png"),
            member(4, 100, 10, "d.png"),
        ];
        let groups = build_groups(
            &members,
            &[
                AcceptedEdge::new(1, 2, 0.92, "sscd"),
                AcceptedEdge::new(3, 4, 0.99, "sscd"),
            ],
        );
        assert_eq!(groups[0].min_similarity, 0.99);
        assert_eq!(groups[1].min_similarity, 0.92);
    }
}
