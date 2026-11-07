//! Finds videos that are probably the same, from their length, size and name.
//! The files aren't read, so a match is a suggestion for the viewer to check.

use std::collections::HashMap;

/// Words that say how a copy differs rather than what the video is.
const NOISE: [&str; 7] = ["副本", "copy", "2160p", "1080p", "720p", "4k", "8k"];

#[derive(Clone, Debug)]
pub struct Item {
    pub id: i64,
    pub name: String,
    pub duration: f64,
    pub size: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Strict,
    Normal,
    Loose,
}

impl Level {
    /// Allowed difference in length, in size, and the least name similarity.
    fn limits(self) -> (f64, f64, f64) {
        match self {
            Self::Strict => (0.005, 0.01, 0.8),
            Self::Normal => (0.02, 0.1, 0.5),
            Self::Loose => (0.05, 0.3, 0.34),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Options {
    pub duration: bool,
    pub size: bool,
    pub name: bool,
    pub level: Level,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Reasons {
    pub duration: bool,
    pub size: bool,
    pub name: bool,
}

impl Reasons {
    fn count(self) -> usize {
        usize::from(self.duration) + usize::from(self.size) + usize::from(self.name)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub ids: Vec<i64>,
    pub reasons: Reasons,
}

impl Group {
    /// All three point the same way.
    pub fn strong(&self) -> bool {
        self.reasons.count() == 3
    }

    /// Stays the same for the same videos, so "not duplicates" can be remembered.
    pub fn key(&self) -> String {
        let mut ids = self.ids.clone();
        ids.sort_unstable();
        ids.iter().map(i64::to_string).collect::<Vec<_>>().join("-")
    }
}

fn drop_numbering(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let open = matches!(chars[i], '(' | '（');
        let close = (i + 1..chars.len()).find(|&j| !chars[j].is_ascii_digit());
        if let (true, Some(end)) = (open, close) {
            if end > i + 1 && matches!(chars[end], ')' | '）') {
                i = end + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn drop_years(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let end = (i..chars.len()).find(|&j| !chars[j].is_ascii_digit()).unwrap_or(chars.len());
            let run: String = chars[i..end].iter().collect();
            if !(run.len() == 4 && (run.starts_with("19") || run.starts_with("20"))) {
                out.push_str(&run);
            }
            i = end;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// A name reduced to what identifies the video.
fn clean_name(name: &str) -> String {
    let mut text = name.to_lowercase();
    for word in NOISE {
        text = text.replace(word, " ");
    }
    drop_years(&drop_numbering(&text)).chars().filter(|c| c.is_alphanumeric()).collect()
}

fn pairs(name: &str) -> HashMap<(char, char), u32> {
    let chars: Vec<char> = name.chars().collect();
    let mut out = HashMap::new();
    for w in chars.windows(2) {
        *out.entry((w[0], w[1])).or_insert(0) += 1;
    }
    if let [only] = chars[..] {
        out.insert((only, only), 1);
    }
    out
}

/// Dice coefficient over pairs of letters: 1 for equal names, 0 for unrelated.
fn similarity(a: &HashMap<(char, char), u32>, b: &HashMap<(char, char), u32>) -> f64 {
    let total: u32 = a.values().sum::<u32>() + b.values().sum::<u32>();
    if total == 0 {
        return 0.0;
    }
    let shared: u32 = a.iter().map(|(k, n)| (*n).min(b.get(k).copied().unwrap_or(0))).sum();
    2.0 * f64::from(shared) / f64::from(total)
}

fn close(a: f64, b: f64, limit: f64) -> bool {
    a > 0.0 && b > 0.0 && (a - b).abs() / a.max(b) <= limit
}

fn root(parent: &mut [usize], i: usize) -> usize {
    let mut r = i;
    while parent[r] != r {
        parent[r] = parent[parent[r]];
        r = parent[r];
    }
    r
}

/// Videos match when two of the chosen tests agree, or the only one chosen.
/// Matches chain, so a group holds everything connected to it. Groups with
/// more evidence come first.
pub fn find(items: &[Item], o: Options) -> Vec<Group> {
    let chosen = usize::from(o.duration) + usize::from(o.size) + usize::from(o.name);
    if chosen == 0 {
        return Vec::new();
    }
    let need = chosen.min(2);
    let (max_duration, max_size, min_name) = o.level.limits();
    let names: Vec<_> = items.iter().map(|v| pairs(&clean_name(&v.name))).collect();
    let mut parent: Vec<usize> = (0..items.len()).collect();
    let mut best: HashMap<usize, Reasons> = HashMap::new();
    let mut links: Vec<(usize, Reasons)> = Vec::new();

    for i in 0..items.len() {
        for j in i + 1..items.len() {
            let (a, b) = (&items[i], &items[j]);
            let mut r = Reasons {
                duration: o.duration && close(a.duration, b.duration, max_duration),
                size: o.size && close(a.size as f64, b.size as f64, max_size),
                name: false,
            };
            // Names cost the most, so they're compared only if they can decide.
            if o.name && r.count() + 1 >= need {
                r.name = similarity(&names[i], &names[j]) >= min_name;
            }
            if r.count() >= need {
                let (x, y) = (root(&mut parent, i), root(&mut parent, j));
                parent[y] = x;
                links.push((i, r));
            }
        }
    }
    for (i, r) in links {
        let top = root(&mut parent, i);
        let entry = best.entry(top).or_default();
        if r.count() > entry.count() {
            *entry = r;
        }
    }

    let mut members: HashMap<usize, Vec<i64>> = HashMap::new();
    for i in 0..items.len() {
        let top = root(&mut parent, i);
        members.entry(top).or_default().push(items[i].id);
    }
    let mut groups: Vec<Group> = members
        .into_iter()
        .filter(|(_, ids)| ids.len() > 1)
        .map(|(top, ids)| Group { ids, reasons: best.get(&top).copied().unwrap_or_default() })
        .collect();
    groups.sort_by(|a, b| b.reasons.count().cmp(&a.reasons.count()).then(a.ids[0].cmp(&b.ids[0])));
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Options = Options { duration: true, size: true, name: true, level: Level::Normal };

    fn v(id: i64, name: &str, duration: f64, gb: f64) -> Item {
        Item { id, name: name.into(), duration, size: (gb * 1e9) as i64 }
    }

    #[test]
    fn names_lose_what_only_marks_a_copy() {
        assert_eq!(clean_name("星际穿越 1080p"), "星际穿越");
        assert_eq!(clean_name("毕业典礼 (1)"), "毕业典礼");
        assert_eq!(clean_name("家庭聚会 副本"), "家庭聚会");
        assert_eq!(clean_name("Trip.2024.4K"), "trip");
        assert_eq!(clean_name("第2集"), "第2集");
    }

    #[test]
    fn similar_names_score_high() {
        let s = |a: &str, b: &str| similarity(&pairs(&clean_name(a)), &pairs(&clean_name(b)));
        assert_eq!(s("毕业典礼", "毕业典礼 (1)"), 1.0);
        assert!(s("东京夜景", "东京夜景 剪辑版") >= 0.5);
        assert!(s("星际穿越", "银翼杀手") < 0.1);
    }

    #[test]
    fn two_agreeing_tests_make_a_match() {
        let items = [v(1, "毕业典礼", 4951.0, 18.2), v(2, "毕业典礼 (1)", 4951.0, 18.1), v(3, "洱海日出", 728.0, 18.7)];
        let g = find(&items, ALL);
        assert_eq!(g.len(), 1);
        assert!(g[0].strong());
        let mut ids = g[0].ids.clone();
        ids.sort_unstable();
        assert_eq!(ids, [1, 2]);
    }

    #[test]
    fn different_names_can_still_match() {
        let g = find(&[v(1, "星际穿越", 10140.0, 58.2), v(2, "Interstellar", 10136.0, 57.8)], ALL);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].reasons, Reasons { duration: true, size: true, name: false });
    }

    #[test]
    fn one_test_alone_is_not_enough_when_more_are_chosen() {
        assert!(find(&[v(1, "甲", 100.0, 1.0), v(2, "乙", 100.0, 9.0)], ALL).is_empty());
    }

    #[test]
    fn a_single_chosen_test_decides() {
        let o = Options { duration: true, size: false, name: false, level: Level::Normal };
        assert_eq!(find(&[v(1, "甲", 100.0, 1.0), v(2, "乙", 100.0, 9.0)], o).len(), 1);
        let none = Options { duration: false, ..o };
        assert!(find(&[v(1, "甲", 100.0, 1.0), v(2, "甲", 100.0, 1.0)], none).is_empty());
    }

    #[test]
    fn stricter_levels_find_less() {
        let items = [v(1, "东京夜景", 1860.0, 33.8), v(2, "东京夜景 剪辑版", 1782.0, 25.0)];
        let at = |level| find(&items, Options { level, ..ALL }).len();
        assert_eq!(at(Level::Strict), 0);
        assert_eq!(at(Level::Loose), 1);
    }

    #[test]
    fn unknown_length_never_matches() {
        assert!(!close(0.0, 0.0, 0.5));
    }

    #[test]
    fn matches_chain_into_one_group() {
        let o = Options { duration: true, size: false, name: false, level: Level::Normal };
        let g = find(&[v(1, "a", 100.0, 1.0), v(2, "b", 101.5, 1.0), v(3, "c", 103.0, 1.0)], o);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].ids.len(), 3);
    }

    #[test]
    fn group_keys_ignore_order() {
        let a = Group { ids: vec![3, 1], reasons: Reasons::default() };
        let b = Group { ids: vec![1, 3], reasons: Reasons::default() };
        assert_eq!(a.key(), b.key());
    }
}
