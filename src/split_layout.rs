//! The multi-view layout: a tree of splits whose leaves are slots, plus
//! floating picture-in-picture slots on top. Pure data, no drawing.
//!
//! A split divides its area side by side (`Dir::H`) or top and bottom
//! (`Dir::V`); `ratio` is the share of the first child. Every node and
//! floating slot has an id from one counter, so ids never collide.

use eframe::egui::{Rect, pos2, vec2};

/// Most videos on screen at once (each is its own decoder).
pub const MAX_SLOTS: usize = 4;
/// Gap between neighbouring slots, in points.
pub const GAP: f32 = 3.0;
const MIN_RATIO: f32 = 0.15;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Content {
    /// The main player (playlist, history, media keys…).
    Main,
    /// An extra player, by id.
    Pane(u32),
    /// Waiting for a video.
    Empty,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    /// Side by side.
    H,
    /// Top and bottom.
    V,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Preset {
    Single,
    LeftRight,
    TopBottom,
    Grid,
    OneBigThree,
    Pip,
}

impl Preset {
    pub const ALL: [Preset; 6] =
        [Preset::Single, Preset::LeftRight, Preset::TopBottom, Preset::Grid, Preset::OneBigThree, Preset::Pip];

    /// The button label, translated.
    pub fn label(self) -> &'static str {
        use crate::i18n::tr;
        match self {
            Preset::Single => tr("Single"),
            Preset::LeftRight => tr("Side by side"),
            Preset::TopBottom => tr("Top and bottom"),
            Preset::Grid => tr("Grid"),
            Preset::OneBigThree => tr("1 + 3"),
            Preset::Pip => tr("Picture-in-picture"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Node {
    Leaf { id: u32, content: Content },
    Split { id: u32, dir: Dir, ratio: f32, a: Box<Node>, b: Box<Node> },
}

impl Node {
    fn id(&self) -> u32 {
        match self {
            Node::Leaf { id, .. } | Node::Split { id, .. } => *id,
        }
    }
}

/// A floating slot; `rect` is in fractions of the area (0..1).
#[derive(Clone, Debug)]
pub struct Float {
    pub id: u32,
    pub content: Content,
    pub rect: Rect,
}

/// A draggable line between the two halves of a split.
#[derive(Clone, Copy, Debug)]
pub struct Divider {
    pub split: u32,
    pub dir: Dir,
    /// Where to draw and grab it.
    pub rect: Rect,
    /// The split's whole area, to turn a pointer position into a ratio.
    pub area: Rect,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub root: Node,
    pub floats: Vec<Float>,
    next: u32,
}

impl Layout {
    pub fn single(content: Content) -> Self {
        Self { root: Node::Leaf { id: 1, content }, floats: Vec::new(), next: 2 }
    }

    fn new_id(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    fn leaf(&mut self, content: Content) -> Node {
        Node::Leaf { id: self.new_id(), content }
    }

    fn split_of(&mut self, dir: Dir, ratio: f32, a: Node, b: Node) -> Node {
        Node::Split { id: self.new_id(), dir, ratio, a: Box::new(a), b: Box::new(b) }
    }

    /// Rebuilds the layout as `preset`, placing `contents` in order (missing
    /// places stay empty). Returns the contents that didn't fit.
    pub fn apply_preset(&mut self, preset: Preset, contents: &[Content]) -> Vec<Content> {
        let mut it = contents.iter().copied().filter(|c| *c != Content::Empty);
        let mut next = || it.next().unwrap_or(Content::Empty);
        self.floats.clear();
        self.root = match preset {
            Preset::Single => self.leaf(next()),
            Preset::LeftRight | Preset::TopBottom => {
                let dir = if preset == Preset::LeftRight { Dir::H } else { Dir::V };
                let (a, b) = (self.leaf(next()), self.leaf(next()));
                self.split_of(dir, 0.5, a, b)
            }
            Preset::Grid => {
                let (a, b, c, d) = (self.leaf(next()), self.leaf(next()), self.leaf(next()), self.leaf(next()));
                let top = self.split_of(Dir::H, 0.5, a, b);
                let bottom = self.split_of(Dir::H, 0.5, c, d);
                self.split_of(Dir::V, 0.5, top, bottom)
            }
            Preset::OneBigThree => {
                let big = self.leaf(next());
                let (b, c, d) = (self.leaf(next()), self.leaf(next()), self.leaf(next()));
                let lower = self.split_of(Dir::V, 0.5, c, d);
                let side = self.split_of(Dir::V, 1.0 / 3.0, b, lower);
                self.split_of(Dir::H, 0.68, big, side)
            }
            Preset::Pip => {
                let main = self.leaf(next());
                let small = next();
                let id = self.new_id();
                self.floats.push(Float { id, content: small, rect: default_float_rect() });
                main
            }
        };
        it.collect()
    }

    /// Every slot's content, leaves first (in reading order), then floats.
    pub fn contents(&self) -> Vec<Content> {
        let mut out: Vec<Content> = self.leaves().into_iter().map(|(_, c)| c).collect();
        out.extend(self.floats.iter().map(|f| f.content));
        out
    }

    pub fn leaves(&self) -> Vec<(u32, Content)> {
        fn walk(n: &Node, out: &mut Vec<(u32, Content)>) {
            match n {
                Node::Leaf { id, content } => out.push((*id, *content)),
                Node::Split { a, b, .. } => {
                    walk(a, out);
                    walk(b, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }

    pub fn slot_count(&self) -> usize {
        self.leaves().len() + self.floats.len()
    }

    pub fn can_add(&self) -> bool {
        self.slot_count() < MAX_SLOTS
    }

    /// Content of a slot (leaf or float).
    pub fn content(&self, id: u32) -> Option<Content> {
        self.leaves()
            .into_iter()
            .find(|(i, _)| *i == id)
            .map(|(_, c)| c)
            .or_else(|| self.floats.iter().find(|f| f.id == id).map(|f| f.content))
    }

    /// The slot showing `content`, if any.
    pub fn find(&self, content: Content) -> Option<u32> {
        self.leaves()
            .into_iter()
            .find(|(_, c)| *c == content)
            .map(|(i, _)| i)
            .or_else(|| self.floats.iter().find(|f| f.content == content).map(|f| f.id))
    }

    pub fn set_content(&mut self, id: u32, content: Content) {
        if let Some(Node::Leaf { content: c, .. }) = find_mut(&mut self.root, id) {
            *c = content;
        } else if let Some(f) = self.floats.iter_mut().find(|f| f.id == id) {
            f.content = content;
        }
    }

    /// Splits a leaf in two; the new half is empty. Returns the new leaf's id.
    pub fn split(&mut self, id: u32, dir: Dir) -> Option<u32> {
        if !self.can_add() {
            return None;
        }
        let content = match find_mut(&mut self.root, id)? {
            Node::Leaf { content, .. } => *content,
            Node::Split { .. } => return None,
        };
        let keep = Node::Leaf { id, content };
        let fresh_id = self.new_id();
        let fresh = Node::Leaf { id: fresh_id, content: Content::Empty };
        let split_id = self.new_id();
        let node = find_mut(&mut self.root, id)?;
        *node = Node::Split { id: split_id, dir, ratio: 0.5, a: Box::new(keep), b: Box::new(fresh) };
        Some(fresh_id)
    }

    /// Removes a slot. A leaf's sibling takes over its space; the last leaf
    /// can't be removed, it just becomes empty. Returns what it showed.
    pub fn remove(&mut self, id: u32) -> Option<Content> {
        if let Some(i) = self.floats.iter().position(|f| f.id == id) {
            return Some(self.floats.remove(i).content);
        }
        let content = self.content(id)?;
        if matches!(self.root, Node::Leaf { .. }) {
            self.set_content(id, Content::Empty);
            return Some(content);
        }
        remove_leaf(&mut self.root, id);
        Some(content)
    }

    /// Swaps what two slots show.
    pub fn swap(&mut self, a: u32, b: u32) {
        if let (Some(ca), Some(cb)) = (self.content(a), self.content(b)) {
            self.set_content(a, cb);
            self.set_content(b, ca);
        }
    }

    /// Turns a leaf into a floating window.
    pub fn float(&mut self, id: u32) -> Option<u32> {
        let content = self.content(id)?;
        if content == Content::Empty || self.floats.iter().any(|f| f.id == id) {
            return None;
        }
        self.remove(id);
        let fid = self.new_id();
        let n = self.floats.len() as f32;
        let mut rect = default_float_rect();
        rect = rect.translate(vec2(-0.04 * n, 0.06 * n));
        self.floats.push(Float { id: fid, content, rect });
        Some(fid)
    }

    /// Puts a floating window back into the tree: into an empty leaf if
    /// there is one, else next to `beside` (or the first leaf).
    pub fn dock(&mut self, float_id: u32, beside: Option<u32>) -> Option<u32> {
        let i = self.floats.iter().position(|f| f.id == float_id)?;
        let content = self.floats.remove(i).content;
        if let Some((leaf, _)) = self.leaves().into_iter().find(|(_, c)| *c == Content::Empty) {
            self.set_content(leaf, content);
            return Some(leaf);
        }
        let target = beside.filter(|b| self.leaves().iter().any(|(i, _)| i == b)).unwrap_or(self.leaves()[0].0);
        // Docking doesn't add a slot, so it's always allowed.
        let fresh_id = self.new_id();
        let split_id = self.new_id();
        let node = find_mut(&mut self.root, target)?;
        let old = std::mem::replace(node, Node::Leaf { id: 0, content: Content::Empty });
        *node = Node::Split {
            id: split_id,
            dir: Dir::H,
            ratio: 0.5,
            a: Box::new(old),
            b: Box::new(Node::Leaf { id: fresh_id, content }),
        };
        Some(fresh_id)
    }

    pub fn set_ratio(&mut self, split: u32, ratio: f32) {
        if let Some(Node::Split { ratio: r, .. }) = find_mut(&mut self.root, split) {
            *r = ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO);
        }
    }

    pub fn set_float_rect(&mut self, id: u32, rect: Rect) {
        if let Some(f) = self.floats.iter_mut().find(|f| f.id == id) {
            let (w, h) = (rect.width().clamp(0.15, 1.0), rect.height().clamp(0.15, 1.0));
            let x = rect.min.x.clamp(0.0, 1.0 - w);
            let y = rect.min.y.clamp(0.0, 1.0 - h);
            f.rect = Rect::from_min_size(pos2(x, y), vec2(w, h));
        }
    }

    /// Where every leaf goes in `area`, and the dividers between them.
    pub fn rects(&self, area: Rect) -> (Vec<(u32, Content, Rect)>, Vec<Divider>) {
        fn walk(n: &Node, r: Rect, leaves: &mut Vec<(u32, Content, Rect)>, divs: &mut Vec<Divider>) {
            match n {
                Node::Leaf { id, content } => leaves.push((*id, *content, r)),
                Node::Split { id, dir, ratio, a, b } => {
                    let (ra, rb, line) = match dir {
                        Dir::H => {
                            let w = (r.width() - GAP) * ratio;
                            let ra = Rect::from_min_size(r.min, vec2(w, r.height()));
                            let rb = Rect::from_min_max(pos2(ra.right() + GAP, r.top()), r.max);
                            let line = Rect::from_min_max(pos2(ra.right() - 2.0, r.top()), pos2(rb.left() + 2.0, r.bottom()));
                            (ra, rb, line)
                        }
                        Dir::V => {
                            let h = (r.height() - GAP) * ratio;
                            let ra = Rect::from_min_size(r.min, vec2(r.width(), h));
                            let rb = Rect::from_min_max(pos2(r.left(), ra.bottom() + GAP), r.max);
                            let line = Rect::from_min_max(pos2(r.left(), ra.bottom() - 2.0), pos2(r.right(), rb.top() + 2.0));
                            (ra, rb, line)
                        }
                    };
                    walk(a, ra, leaves, divs);
                    walk(b, rb, leaves, divs);
                    divs.push(Divider { split: *id, dir: *dir, rect: line, area: r });
                }
            }
        }
        let (mut leaves, mut divs) = (Vec::new(), Vec::new());
        walk(&self.root, area, &mut leaves, &mut divs);
        (leaves, divs)
    }

    /// A floating slot's rect in `area`.
    pub fn float_rect(f: &Float, area: Rect) -> Rect {
        Rect::from_min_size(
            area.min + vec2(f.rect.min.x * area.width(), f.rect.min.y * area.height()),
            vec2(f.rect.width() * area.width(), f.rect.height() * area.height()),
        )
    }
}

fn default_float_rect() -> Rect {
    Rect::from_min_size(pos2(0.62, 0.06), vec2(0.34, 0.34))
}

fn find_mut(n: &mut Node, id: u32) -> Option<&mut Node> {
    if n.id() == id {
        return Some(n);
    }
    match n {
        Node::Leaf { .. } => None,
        Node::Split { a, b, .. } => find_mut(a, id).or_else(move || find_mut(b, id)),
    }
}

/// Replaces the split holding leaf `id` with the leaf's sibling.
fn remove_leaf(n: &mut Node, id: u32) -> bool {
    let Node::Split { a, b, .. } = n else { return false };
    if a.id() == id {
        let sibling = std::mem::replace(&mut **b, Node::Leaf { id: 0, content: Content::Empty });
        *n = sibling;
        return true;
    }
    if b.id() == id {
        let sibling = std::mem::replace(&mut **a, Node::Leaf { id: 0, content: Content::Empty });
        *n = sibling;
        return true;
    }
    remove_leaf(a, id) || remove_leaf(b, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0))
    }

    #[test]
    fn split_close_and_limits() {
        let mut l = Layout::single(Content::Main);
        let root = l.leaves()[0].0;
        let right = l.split(root, Dir::H).unwrap();
        assert_eq!(l.contents(), [Content::Main, Content::Empty]);
        l.set_content(right, Content::Pane(7));
        let below = l.split(right, Dir::V).unwrap();
        let third = l.split(below, Dir::H).unwrap();
        assert_eq!(l.slot_count(), 4);
        assert!(l.split(third, Dir::H).is_none(), "capped at MAX_SLOTS");

        assert_eq!(l.remove(right), Some(Content::Pane(7)));
        assert_eq!(l.slot_count(), 3);
        // The last leaf can't go away; it just empties.
        l.remove(below);
        l.remove(third);
        assert_eq!(l.contents(), [Content::Main]);
        l.remove(root);
        assert_eq!(l.contents(), [Content::Empty]);
    }

    #[test]
    fn rects_tile_the_area_and_ratio_moves_the_divider() {
        let mut l = Layout::single(Content::Main);
        let root = l.leaves()[0].0;
        l.split(root, Dir::H);
        let (leaves, divs) = l.rects(area());
        assert_eq!(leaves.len(), 2);
        assert_eq!(divs.len(), 1);
        let (a, b) = (leaves[0].2, leaves[1].2);
        assert!((a.width() + b.width() + GAP - 1000.0).abs() < 0.01);
        l.set_ratio(divs[0].split, 0.7);
        let (leaves, _) = l.rects(area());
        assert!((leaves[0].2.width() - (1000.0 - GAP) * 0.7).abs() < 0.01);
        l.set_ratio(divs[0].split, 0.99);
        let (leaves, _) = l.rects(area());
        assert!(leaves[1].2.width() >= (1000.0 - GAP) * MIN_RATIO - 0.01, "can't squeeze a slot away");
    }

    #[test]
    fn presets_keep_order_and_report_leftovers() {
        let mut l = Layout::single(Content::Main);
        let all = [Content::Main, Content::Pane(1), Content::Pane(2), Content::Pane(3)];
        assert!(l.apply_preset(Preset::Grid, &all).is_empty());
        assert_eq!(l.contents(), all);
        let rest = l.apply_preset(Preset::LeftRight, &all);
        assert_eq!(l.contents(), [Content::Main, Content::Pane(1)]);
        assert_eq!(rest, [Content::Pane(2), Content::Pane(3)]);
        l.apply_preset(Preset::Pip, &[Content::Main, Content::Pane(1)]);
        assert_eq!(l.leaves().len(), 1);
        assert_eq!(l.floats.len(), 1);
        assert_eq!(l.contents(), [Content::Main, Content::Pane(1)]);
    }

    #[test]
    fn swap_float_and_dock() {
        let mut l = Layout::single(Content::Main);
        l.apply_preset(Preset::LeftRight, &[Content::Main, Content::Pane(1)]);
        let ids: Vec<u32> = l.leaves().iter().map(|(i, _)| *i).collect();
        l.swap(ids[0], ids[1]);
        assert_eq!(l.contents(), [Content::Pane(1), Content::Main]);

        let fid = l.float(ids[1]).unwrap();
        assert_eq!(l.leaves().len(), 1, "the floated leaf's space goes to its sibling");
        assert_eq!(l.content(fid), Some(Content::Main));
        let back = l.dock(fid, None).unwrap();
        assert_eq!(l.content(back), Some(Content::Main));
        assert_eq!(l.leaves().len(), 2);
        assert!(l.floats.is_empty());
    }
}
