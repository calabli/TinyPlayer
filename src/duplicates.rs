//! Finds videos that are probably the same, from their length, size and name.
//! The files aren't read, so a match is a suggestion for the viewer to check.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Words that say how a copy differs rather than what the video is.
/// The first is Chinese for "copy", as in "name copy.mp4".
const NOISE: [&str; 7] = ["\u{526F}\u{672C}", "copy", "2160p", "1080p", "720p", "4k", "8k"];

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
        // Full-width brackets too, as Chinese text uses them.
        let open = matches!(chars[i], '(' | '\u{FF08}');
        let close = (i + 1..chars.len()).find(|&j| !chars[j].is_ascii_digit());
        if let (true, Some(end)) = (open, close)
            && end > i + 1 && matches!(chars[end], ')' | '\u{FF09}') {
                i = end + 1;
                continue;
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

/// Pairs of letters in a name, as a set.
fn pairs(name: &str) -> Vec<(char, char)> {
    let chars: Vec<char> = name.chars().collect();
    let mut out: Vec<(char, char)> = chars.windows(2).map(|w| (w[0], w[1])).collect();
    if let [only] = chars[..] {
        out.push((only, only));
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Numbers each name's letter pairs, rarest overall first, and sorts them.
/// Comparing sorted numbers is much faster than comparing text, and having
/// the rare pairs first lets `name_matches` look at only a few of them.
fn rank_names(items: &[Item]) -> Vec<Vec<u32>> {
    let names: Vec<_> = items.iter().map(|v| pairs(&clean_name(&v.name))).collect();
    let mut counts: HashMap<(char, char), u32> = HashMap::new();
    for pair in names.iter().flatten() {
        *counts.entry(*pair).or_insert(0) += 1;
    }
    let mut order: Vec<_> = counts.into_iter().collect();
    order.sort_unstable_by_key(|(pair, count)| (*count, *pair));
    let rank: HashMap<(char, char), u32> = order.into_iter().enumerate().map(|(i, (pair, _))| (pair, i as u32)).collect();
    names
        .into_iter()
        .map(|name| {
            let mut ranks: Vec<u32> = name.iter().map(|pair| rank[pair]).collect();
            ranks.sort_unstable();
            ranks
        })
        .collect()
}

/// Dice coefficient over pairs of letters: 1 for equal names, 0 for unrelated.
fn similarity(a: &[u32], b: &[u32]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let (mut i, mut j, mut shared) = (0, 0, 0u32);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                shared += 1;
                i += 1;
                j += 1;
            }
        }
    }
    2.0 * f64::from(shared) / (a.len() + b.len()) as f64
}

fn name_matches(a: &[u32], b: &[u32], least: f64) -> bool {
    // Names too different in length can't reach `least`, however they overlap.
    let (short, long) = if a.len() < b.len() { (a.len(), b.len()) } else { (b.len(), a.len()) };
    2.0 * short as f64 / (short + long) as f64 >= least && similarity(a, b) >= least
}

/// How many of a name's rarest pairs must include one it shares with any
/// name similar to it: the usual prefix filter for set similarity joins.
fn prefix_len(len: usize, least: f64) -> usize {
    let overlap = (len as f64 * least / (2.0 - least) - 1e-9).ceil().max(1.0) as usize;
    (len + 1).saturating_sub(overlap).min(len)
}

fn close(a: f64, b: f64, limit: f64) -> bool {
    a > 0.0 && b > 0.0 && (a - b).abs() / a.max(b) <= limit
}

fn root(parent: &mut [u32], i: usize) -> usize {
    let mut r = i;
    while parent[r] as usize != r {
        let up = parent[r] as usize;
        parent[r] = parent[up];
        r = parent[r] as usize;
    }
    r
}

/// How far a comparison has got, for showing and for stopping it.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    pub cancel: AtomicBool,
}

impl Progress {
    pub fn fraction(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed).max(1);
        self.done.load(Ordering::Relaxed) as f32 / total as f32
    }
}

struct Search<'a> {
    items: &'a [Item],
    names: Vec<Vec<u32>>,
    o: Options,
    need: usize,
    limits: (f64, f64, f64),
}

type Link = (u32, u32, Reasons);

impl Search<'_> {
    fn compare(&self, i: usize, j: usize) -> Reasons {
        let (a, b) = (&self.items[i], &self.items[j]);
        let (max_duration, max_size, min_name) = self.limits;
        let mut r = Reasons {
            duration: self.o.duration && close(a.duration, b.duration, max_duration),
            size: self.o.size && close(a.size as f64, b.size as f64, max_size),
            name: false,
        };
        // Names cost the most, so they're compared only if they can decide.
        if self.o.name && r.count() + 1 >= self.need {
            r.name = name_matches(&self.names[i], &self.names[j], min_name);
        }
        r
    }

    /// Items in order of a measure, so only neighbors need comparing.
    fn sorted_by(&self, key: impl Fn(&Item) -> f64) -> (Vec<u32>, Vec<f64>) {
        let mut order: Vec<u32> = (0..self.items.len() as u32).filter(|i| key(&self.items[*i as usize]) > 0.0).collect();
        order.sort_unstable_by(|a, b| key(&self.items[*a as usize]).total_cmp(&key(&self.items[*b as usize])));
        let keys = order.iter().map(|i| key(&self.items[*i as usize])).collect();
        (order, keys)
    }

    /// Runs `work` for each position on every core. Workers keep only the
    /// links that join groups or improve one, so memory stays small however
    /// many videos look alike.
    fn in_parallel(&self, positions: usize, p: &Progress, work: impl Fn(usize, &mut dyn FnMut(usize, usize)) + Sync) -> Vec<Link> {
        const CHUNK: usize = 64;
        let next = AtomicUsize::new(0);
        let threads = std::thread::available_parallelism().map_or(1, usize::from).clamp(1, 8);
        std::thread::scope(|scope| {
            let workers: Vec<_> = (0..threads)
                .map(|_| {
                    scope.spawn(|| {
                        let mut parent: Vec<u32> = (0..self.items.len() as u32).collect();
                        let mut best = vec![0u8; self.items.len()];
                        let mut links = Vec::new();
                        loop {
                            crate::background::wait(&p.cancel);
                            let first = next.fetch_add(CHUNK, Ordering::Relaxed);
                            if first >= positions || p.cancel.load(Ordering::Relaxed) {
                                return links;
                            }
                            let last = (first + CHUNK).min(positions);
                            let mut visit = |i: usize, j: usize| {
                                let r = self.compare(i, j);
                                if r.count() < self.need {
                                    return;
                                }
                                let (x, y) = (root(&mut parent, i), root(&mut parent, j));
                                let count = r.count() as u8;
                                if x == y && count <= best[x] {
                                    return;
                                }
                                parent[y] = x as u32;
                                best[x] = best[x].max(best[y]).max(count);
                                links.push((i as u32, j as u32, r));
                            };
                            for position in first..last {
                                work(position, &mut visit);
                            }
                            p.done.fetch_add(last - first, Ordering::Relaxed);
                        }
                    })
                })
                .collect();
            workers.into_iter().flat_map(|w| w.join().unwrap_or_default()).collect()
        })
    }

    fn sweep(&self, key: impl Fn(&Item) -> f64, limit: f64, skip: impl Fn(usize, usize) -> bool + Sync, p: &Progress) -> Vec<Link> {
        let (order, keys) = self.sorted_by(key);
        self.in_parallel(order.len(), p, |position, visit| {
            let i = order[position] as usize;
            for next in position + 1..order.len() {
                if !close(keys[position], keys[next], limit) {
                    break;
                }
                let j = order[next] as usize;
                if !skip(i, j) {
                    visit(i, j);
                }
            }
        })
    }

    fn by_name(&self, p: &Progress) -> Vec<Link> {
        let least = self.limits.2;
        let mut index: HashMap<u32, Vec<u32>> = HashMap::new();
        for (i, name) in self.names.iter().enumerate() {
            for rank in &name[..prefix_len(name.len(), least)] {
                index.entry(*rank).or_default().push(i as u32);
            }
        }
        self.in_parallel(self.items.len(), p, |i, visit| {
            let name = &self.names[i];
            let mut seen = HashSet::new();
            for rank in &name[..prefix_len(name.len(), least)] {
                for &j in index.get(rank).into_iter().flatten() {
                    if j as usize > i && seen.insert(j) {
                        visit(i, j as usize);
                    }
                }
            }
        })
    }
}

/// [`find_with`], when nobody's watching.
#[cfg(test)]
pub fn find(items: &[Item], o: Options) -> Vec<Group> {
    find_with(items, o, &Progress::default())
}

/// Videos match when two of the chosen tests agree, or the only one chosen.
/// Matches chain, so a group holds everything connected to it. Groups with
/// more evidence come first.
pub fn find_with(items: &[Item], o: Options, p: &Progress) -> Vec<Group> {
    let chosen = usize::from(o.duration) + usize::from(o.size) + usize::from(o.name);
    if chosen == 0 {
        return Vec::new();
    }
    let search = Search { items, names: if o.name { rank_names(items) } else { vec![Vec::new(); items.len()] }, o, need: chosen.min(2), limits: o.level.limits() };
    let (max_duration, max_size, _) = search.limits;
    let passes = usize::from(o.duration) + usize::from(o.size) + usize::from(o.name && !o.duration && !o.size);
    p.total.store(items.len() * passes, Ordering::Relaxed);

    // Two tests must agree, and with three chosen one of them is always the
    // length or the size, so these passes meet every possible pair.
    let mut links = Vec::new();
    if o.duration {
        links.extend(search.sweep(|v| v.duration, max_duration, |_, _| false, p));
    }
    if o.size {
        let seen = |i: usize, j: usize| o.duration && close(items[i].duration, items[j].duration, max_duration);
        links.extend(search.sweep(|v| v.size as f64, max_size, seen, p));
    }
    if o.name && !o.duration && !o.size {
        links.extend(search.by_name(p));
    }
    if p.cancel.load(Ordering::Relaxed) {
        return Vec::new();
    }

    let mut parent: Vec<u32> = (0..items.len() as u32).collect();
    for (i, j, _) in &links {
        let (x, y) = (root(&mut parent, *i as usize), root(&mut parent, *j as usize));
        parent[y] = x as u32;
    }
    let mut best: HashMap<usize, Reasons> = HashMap::new();
    for (i, _, r) in links {
        let top = root(&mut parent, i as usize);
        let entry = best.entry(top).or_default();
        if r.count() > entry.count() {
            *entry = r;
        }
    }
    let mut members: HashMap<usize, Vec<i64>> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        members.entry(root(&mut parent, i)).or_default().push(item.id);
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
        assert_eq!(clean_name("\u{661F}\u{9645}\u{7A7F}\u{8D8A} 1080p"), "\u{661F}\u{9645}\u{7A7F}\u{8D8A}");
        assert_eq!(clean_name("\u{6BD5}\u{4E1A}\u{5178}\u{793C} (1)"), "\u{6BD5}\u{4E1A}\u{5178}\u{793C}");
        assert_eq!(clean_name("\u{5BB6}\u{5EAD}\u{805A}\u{4F1A} \u{526F}\u{672C}"), "\u{5BB6}\u{5EAD}\u{805A}\u{4F1A}");
        assert_eq!(clean_name("Trip.2024.4K"), "trip");
        assert_eq!(clean_name("\u{7B2C}2\u{96C6}"), "\u{7B2C}2\u{96C6}");
    }

    #[test]
    fn similar_names_score_high() {
        let s = |a: &str, b: &str| {
            let items = [v(1, a, 0.0, 0.0), v(2, b, 0.0, 0.0)];
            let names = rank_names(&items);
            similarity(&names[0], &names[1])
        };
        assert_eq!(s("\u{6BD5}\u{4E1A}\u{5178}\u{793C}", "\u{6BD5}\u{4E1A}\u{5178}\u{793C} (1)"), 1.0);
        assert!(s("\u{4E1C}\u{4EAC}\u{591C}\u{666F}", "\u{4E1C}\u{4EAC}\u{591C}\u{666F} \u{526A}\u{8F91}\u{7248}") >= 0.5);
        assert!(s("\u{661F}\u{9645}\u{7A7F}\u{8D8A}", "\u{94F6}\u{7FFC}\u{6740}\u{624B}") < 0.1);
    }

    #[test]
    fn two_agreeing_tests_make_a_match() {
        let items = [v(1, "\u{6BD5}\u{4E1A}\u{5178}\u{793C}", 4951.0, 18.2), v(2, "\u{6BD5}\u{4E1A}\u{5178}\u{793C} (1)", 4951.0, 18.1), v(3, "\u{6D31}\u{6D77}\u{65E5}\u{51FA}", 728.0, 18.7)];
        let g = find(&items, ALL);
        assert_eq!(g.len(), 1);
        assert!(g[0].strong());
        let mut ids = g[0].ids.clone();
        ids.sort_unstable();
        assert_eq!(ids, [1, 2]);
    }

    #[test]
    fn different_names_can_still_match() {
        let g = find(&[v(1, "\u{661F}\u{9645}\u{7A7F}\u{8D8A}", 10140.0, 58.2), v(2, "Interstellar", 10136.0, 57.8)], ALL);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].reasons, Reasons { duration: true, size: true, name: false });
    }

    #[test]
    fn one_test_alone_is_not_enough_when_more_are_chosen() {
        assert!(find(&[v(1, "\u{7532}", 100.0, 1.0), v(2, "\u{4E59}", 100.0, 9.0)], ALL).is_empty());
    }

    #[test]
    fn a_single_chosen_test_decides() {
        let o = Options { duration: true, size: false, name: false, level: Level::Normal };
        assert_eq!(find(&[v(1, "\u{7532}", 100.0, 1.0), v(2, "\u{4E59}", 100.0, 9.0)], o).len(), 1);
        let none = Options { duration: false, ..o };
        assert!(find(&[v(1, "\u{7532}", 100.0, 1.0), v(2, "\u{7532}", 100.0, 1.0)], none).is_empty());
    }

    #[test]
    fn stricter_levels_find_less() {
        let items = [v(1, "\u{4E1C}\u{4EAC}\u{591C}\u{666F}", 1860.0, 33.8), v(2, "\u{4E1C}\u{4EAC}\u{591C}\u{666F} \u{526A}\u{8F91}\u{7248}", 1782.0, 25.0)];
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

    /// The sweep and the name index must find what comparing all pairs would.
    #[test]
    fn the_shortcuts_find_the_same_groups_as_comparing_everything() {
        let titles = ["\u{661F}\u{9645}\u{7A7F}\u{8D8A}", "\u{661F}\u{9645}\u{7A7F}\u{8D8A} \u{5BFC}\u{6F14}\u{526A}\u{8F91}", "\u{94F6}\u{7FFC}\u{6740}\u{624B}", "\u{94F6}\u{7FFC}\u{6740}\u{624B} 2049", "\u{4E1C}\u{4EAC}\u{591C}\u{666F}", "\u{4E1C}\u{4EAC}\u{591C}\u{666F} \u{526A}\u{8F91}\u{7248}", "\u{6BD5}\u{4E1A}\u{5178}\u{793C}", "\u{6BD5}\u{4E1A}", "a", "ab", "\u{7B2C}\u{4E00}\u{96C6}", "\u{7B2C}\u{4E8C}\u{96C6}"];
        let items: Vec<Item> = (0..240)
            .map(|i| {
                let n = i as usize;
                v(i, &format!("{} {}", titles[n % titles.len()], n % 5), 100.0 + (n * 37 % 900) as f64, 1.0 + (n * 53 % 700) as f64 / 10.0)
            })
            .collect();
        let names = rank_names(&items);
        for level in [Level::Strict, Level::Normal, Level::Loose] {
            for mask in 1..8 {
                let o = Options { duration: mask & 1 != 0, size: mask & 2 != 0, name: mask & 4 != 0, level };
                let chosen = (mask as u32).count_ones() as usize;
                let (d, g, n) = level.limits();
                let mut parent: Vec<u32> = (0..items.len() as u32).collect();
                for i in 0..items.len() {
                    for j in i + 1..items.len() {
                        let hits = usize::from(o.duration && close(items[i].duration, items[j].duration, d))
                            + usize::from(o.size && close(items[i].size as f64, items[j].size as f64, g))
                            + usize::from(o.name && similarity(&names[i], &names[j]) >= n);
                        if hits >= chosen.min(2) {
                            let (x, y) = (root(&mut parent, i), root(&mut parent, j));
                            parent[y] = x as u32;
                        }
                    }
                }
                let mut expected: HashMap<usize, Vec<i64>> = HashMap::new();
                for (i, item) in items.iter().enumerate() {
                    expected.entry(root(&mut parent, i)).or_default().push(item.id);
                }
                let mut expected: Vec<Vec<i64>> = expected.into_values().filter(|g| g.len() > 1).collect();
                let mut found: Vec<Vec<i64>> = find(&items, o).into_iter().map(|g| g.ids).collect();
                for g in expected.iter_mut().chain(found.iter_mut()) {
                    g.sort_unstable();
                }
                expected.sort();
                found.sort();
                assert_eq!(found, expected, "{o:?}");
            }
        }
    }

    #[test]
    fn a_cancelled_search_finds_nothing() {
        let p = Progress::default();
        p.cancel.store(true, Ordering::Relaxed);
        assert!(find_with(&[v(1, "a", 100.0, 1.0), v(2, "a", 100.0, 1.0)], ALL, &p).is_empty());
    }

    #[test]
    fn group_keys_ignore_order() {
        let a = Group { ids: vec![3, 1], reasons: Reasons::default() };
        let b = Group { ids: vec![1, 3], reasons: Reasons::default() };
        assert_eq!(a.key(), b.key());
    }
}
