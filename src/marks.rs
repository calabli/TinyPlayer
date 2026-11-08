//! Places to jump to in the current file: its chapters and the viewer's
//! bookmarks.

use crate::history::Bookmark;
use crate::i18n::tr;

/// Going back this soon after a chapter starts means the previous chapter;
/// later than this, it means the start of the current one.
const RESTART_SECS: f64 = 3.0;
/// A chapter this close ahead is the one we're already arriving at.
const NEXT_MARGIN_SECS: f64 = 0.5;
/// A second bookmark this close to an existing one is the same place.
const SAME_PLACE_SECS: f64 = 1.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Chapter {
    pub time: f64,
    pub title: String,
}

/// One row of the marks list.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub time: f64,
    pub title: String,
    /// Set for bookmarks, which can be removed.
    pub bookmark: Option<i64>,
}

/// Marks of the file that's playing.
#[derive(Default)]
pub struct Marks {
    /// Identifies the file in the bookmarks table; None for URLs.
    pub key: Option<String>,
    pub chapters: Vec<Chapter>,
    pub bookmarks: Vec<Bookmark>,
}

impl Marks {
    pub fn len(&self) -> usize {
        self.chapters.len() + self.bookmarks.len()
    }

    /// Index of the chapter playing at `pos`.
    pub fn chapter_at(&self, pos: f64) -> Option<usize> {
        self.chapters.iter().rposition(|c| c.time <= pos)
    }

    pub fn next_chapter(&self, pos: f64) -> Option<usize> {
        self.chapters.iter().position(|c| c.time > pos + NEXT_MARGIN_SECS)
    }

    pub fn prev_chapter(&self, pos: f64) -> Option<usize> {
        let cur = self.chapter_at(pos)?;
        if pos - self.chapters[cur].time > RESTART_SECS { Some(cur) } else { Some(cur.saturating_sub(1)) }
    }

    pub fn has_bookmark_near(&self, pos: f64) -> bool {
        self.bookmarks.iter().any(|b| (b.time - pos).abs() < SAME_PLACE_SECS)
    }

    /// Chapters and bookmarks together, in time order.
    pub fn rows(&self) -> Vec<Row> {
        let chapters = self.chapters.iter().map(|c| Row { time: c.time, title: c.title.clone(), bookmark: None });
        let bookmarks = self.bookmarks.iter().map(|b| Row {
            time: b.time,
            title: if b.label.is_empty() { tr("Bookmark").to_string() } else { b.label.clone() },
            bookmark: Some(b.id),
        });
        let mut rows: Vec<Row> = chapters.chain(bookmarks).collect();
        rows.sort_by(|a, b| a.time.total_cmp(&b.time));
        rows
    }
}

/// Index of the row playing at `pos` in time-ordered `rows`.
pub fn current_row(rows: &[Row], pos: f64) -> Option<usize> {
    rows.iter().rposition(|r| r.time <= pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapters() -> Marks {
        let chapters = [0.0, 60.0, 120.0].iter().map(|t| Chapter { time: *t, title: format!("{t}") }).collect();
        Marks { chapters, ..Default::default() }
    }

    fn bookmark(id: i64, time: f64, label: &str) -> Bookmark {
        Bookmark { id, time, label: label.to_string() }
    }

    #[test]
    fn next_chapter_skips_one_we_are_arriving_at() {
        let m = chapters();
        assert_eq!(m.next_chapter(10.0), Some(1));
        assert_eq!(m.next_chapter(59.8), Some(2));
        assert_eq!(m.next_chapter(130.0), None);
    }

    #[test]
    fn prev_chapter_restarts_then_goes_back() {
        let m = chapters();
        assert_eq!(m.prev_chapter(70.0), Some(1));
        assert_eq!(m.prev_chapter(61.0), Some(0));
        assert_eq!(m.prev_chapter(1.0), Some(0));
    }

    #[test]
    fn no_chapters_means_no_jumps() {
        let m = Marks::default();
        assert_eq!(m.next_chapter(0.0), None);
        assert_eq!(m.prev_chapter(10.0), None);
        assert_eq!(m.chapter_at(10.0), None);
    }

    #[test]
    fn rows_merge_in_time_order_and_name_unlabeled_bookmarks() {
        let mut m = chapters();
        m.bookmarks = vec![bookmark(7, 90.0, ""), bookmark(8, 30.0, "\u{5F00}\u{573A}")];
        let rows = m.rows();
        let times: Vec<f64> = rows.iter().map(|r| r.time).collect();
        assert_eq!(times, [0.0, 30.0, 60.0, 90.0, 120.0]);
        assert_eq!(rows[1].title, "\u{5F00}\u{573A}");
        assert_eq!(rows[3].title, crate::i18n::tr("Bookmark"));
        assert_eq!(rows[3].bookmark, Some(7));
        assert_eq!(current_row(&rows, 95.0), Some(3));
    }

    #[test]
    fn nearby_bookmark_counts_as_the_same_place() {
        let m = Marks { bookmarks: vec![bookmark(1, 30.0, "")], ..Default::default() };
        assert!(m.has_bookmark_near(30.5));
        assert!(!m.has_bookmark_near(32.0));
    }
}
