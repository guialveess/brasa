//! Lane layout of a commit graph. Pure, no IO.
//!
//! Feed the commits in the order `git log --topo-order` prints them (children before parents)
//! to [`GraphLayout::push`], one at a time, and draw each returned [`GraphRow`]. The layout keeps
//! the active lanes between calls, so a log loaded in pages is laid out by pushing the next
//! page's commits into the same [`GraphLayout`].
//!
//! A *lane* is a column that waits for one commit further down. Each row is split in two halves
//! by the dot of its commit: the top half draws what comes into the dot, the bottom half what
//! goes out of it. Every step is O(lanes).

/// Which half of a row a [`GraphLine`] is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinePart {
    /// From the top edge of the row (column `from`) to the center, where the dot is (column
    /// `to`).
    Top,
    /// From the center, where the dot is (column `from`), to the bottom edge of the row
    /// (column `to`).
    Bottom,
}

/// One segment of a row. A straight line has `from == to`, otherwise it is a curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphLine {
    /// The column at the start of the segment (the edge of the row for [`LinePart::Top`], the
    /// center for [`LinePart::Bottom`]).
    pub from: usize,
    /// The column at the end of the segment.
    pub to: usize,
    /// The color index of the lane this segment belongs to.
    pub color: usize,
    /// The half of the row it is drawn in.
    pub part: LinePart,
}

/// The layout of one commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRow {
    /// The column of the commit's dot.
    pub lane: usize,
    /// The color index of the dot's lane. It is stable along a lane, a counter assigned when the
    /// lane starts (take it modulo the size of the palette).
    pub color: usize,
    /// The segments to draw. Lanes passing through the row have a [`LinePart::Top`] and a
    /// [`LinePart::Bottom`] segment, both straight.
    pub lines: Vec<GraphLine>,
    /// The number of columns this row uses, for the width of the graph column.
    pub width: usize,
}

/// A lane waiting for a commit.
#[derive(Debug, Clone)]
struct Lane {
    /// The full id of the commit this lane leads to.
    waiting_for: String,
    color: usize,
}

/// The state of the lanes between two commits. See the module documentation.
#[derive(Debug, Clone, Default)]
pub struct GraphLayout {
    /// The columns, `None` for a free one. Never ends with a free column.
    lanes: Vec<Option<Lane>>,
    next_color: usize,
}

impl GraphLayout {
    /// An empty layout, for the first page.
    pub fn new() -> GraphLayout {
        GraphLayout::default()
    }

    /// Lays out the next commit `oid` with its `parents` (full ids, first parent first) and
    /// returns its row.
    ///
    /// Lanes waiting for a commit that is never pushed (the log was cut by a page or a maximum
    /// count) just continue down.
    pub fn push(&mut self, oid: &str, parents: &[String]) -> GraphRow {
        let mut lines = Vec::new();

        // The lanes that lead to this commit. The leftmost one holds the dot, the others
        // converge into it. A commit nobody waits for (a branch tip) opens a new lane.
        let incoming: Vec<usize> = self
            .lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| lane.as_ref().is_some_and(|l| l.waiting_for == oid))
            .map(|(i, _)| i)
            .collect();
        let (lane, color) = match incoming.first() {
            Some(&first) => (first, self.color_of(first)),
            None => (self.free_slot(), self.new_color()),
        };
        let mut width = lane + 1;

        for &i in &incoming {
            lines.push(GraphLine {
                from: i,
                to: lane,
                color: self.color_of(i),
                part: LinePart::Top,
            });
            width = width.max(i + 1);
        }
        // Lanes that go past this commit, straight through both halves.
        for (i, slot) in self.lanes.iter().enumerate() {
            if let Some(other) = slot
                && !incoming.contains(&i)
            {
                for part in [LinePart::Top, LinePart::Bottom] {
                    lines.push(GraphLine {
                        from: i,
                        to: i,
                        color: other.color,
                        part,
                    });
                }
                width = width.max(i + 1);
            }
        }

        // The converged lanes end here; the dot's lane goes on to the first parent.
        for &i in &incoming {
            self.lanes[i] = None;
        }
        if let Some(first) = parents.first() {
            self.set_lane(
                lane,
                Lane {
                    waiting_for: first.clone(),
                    color,
                },
            );
            lines.push(GraphLine {
                from: lane,
                to: lane,
                color,
                part: LinePart::Bottom,
            });
        }
        // Every other parent joins the lane already leading to it, or opens a new one.
        for parent in parents.iter().skip(1) {
            let existing = self
                .lanes
                .iter()
                .position(|l| l.as_ref().is_some_and(|l| l.waiting_for == *parent));
            let (target, line_color) = match existing {
                Some(target) => (target, self.color_of(target)),
                None => {
                    let target = self.free_slot();
                    let line_color = self.new_color();
                    self.set_lane(
                        target,
                        Lane {
                            waiting_for: parent.clone(),
                            color: line_color,
                        },
                    );
                    (target, line_color)
                }
            };
            if target != lane {
                lines.push(GraphLine {
                    from: lane,
                    to: target,
                    color: line_color,
                    part: LinePart::Bottom,
                });
            }
            width = width.max(target + 1);
        }
        while matches!(self.lanes.last(), Some(None)) {
            self.lanes.pop();
        }

        GraphRow {
            lane,
            color,
            lines,
            width,
        }
    }

    /// The number of columns currently in use (the highest occupied column plus one).
    pub fn width(&self) -> usize {
        self.lanes.len()
    }

    fn color_of(&self, index: usize) -> usize {
        self.lanes
            .get(index)
            .and_then(|l| l.as_ref())
            .map_or(0, |l| l.color)
    }

    fn new_color(&mut self) -> usize {
        let color = self.next_color;
        self.next_color += 1;
        color
    }

    /// The leftmost free column, a new one at the end when all are taken. It is not reserved
    /// until [`GraphLayout::set_lane`] fills it.
    fn free_slot(&self) -> usize {
        self.lanes
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.lanes.len())
    }

    fn set_lane(&mut self, index: usize, lane: Lane) {
        if index >= self.lanes.len() {
            self.lanes.resize(index + 1, None);
        }
        self.lanes[index] = Some(lane);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn line(from: usize, to: usize, color: usize, part: LinePart) -> GraphLine {
        GraphLine {
            from,
            to,
            color,
            part,
        }
    }

    fn has(row: &GraphRow, expected: GraphLine) -> bool {
        row.lines.contains(&expected)
    }

    #[test]
    fn linear_history_stays_in_lane_zero() {
        let mut layout = GraphLayout::new();
        let rows = [
            layout.push("c", &ids(&["b"])),
            layout.push("b", &ids(&["a"])),
            layout.push("a", &[]),
        ];
        for row in &rows {
            assert_eq!((row.lane, row.color, row.width), (0, 0, 1));
        }
        // A tip has nothing coming in, only the line to its parent.
        assert_eq!(rows[0].lines, vec![line(0, 0, 0, LinePart::Bottom)]);
        // A middle commit has both halves.
        assert_eq!(
            rows[1].lines,
            vec![
                line(0, 0, 0, LinePart::Top),
                line(0, 0, 0, LinePart::Bottom)
            ]
        );
        // The root has nothing going out.
        assert_eq!(rows[2].lines, vec![line(0, 0, 0, LinePart::Top)]);
        assert_eq!(layout.width(), 0);
    }

    #[test]
    fn branch_and_merge() {
        // M merges C (side branch, from A) into B. History: M(B, C), B(A), C(A), A.
        let mut layout = GraphLayout::new();
        let m = layout.push("M", &ids(&["B", "C"]));
        assert_eq!((m.lane, m.width), (0, 2));
        assert!(has(&m, line(0, 0, 0, LinePart::Bottom)));
        // The second parent opens lane 1 with a new color, drawn as a curve in the bottom half.
        assert!(has(&m, line(0, 1, 1, LinePart::Bottom)));

        let c = layout.push("C", &ids(&["A"]));
        // C is in lane 1; lane 0 passes through.
        assert_eq!((c.lane, c.color, c.width), (1, 1, 2));
        assert!(has(&c, line(0, 0, 0, LinePart::Top)));
        assert!(has(&c, line(0, 0, 0, LinePart::Bottom)));
        assert!(has(&c, line(1, 1, 1, LinePart::Top)));
        assert!(has(&c, line(1, 1, 1, LinePart::Bottom)));

        let b = layout.push("B", &ids(&["A"]));
        assert_eq!((b.lane, b.width), (0, 2));
        // Lane 1 passes through B while it waits for A as well.
        assert!(has(&b, line(1, 1, 1, LinePart::Bottom)));

        // A is awaited by both lanes: lane 1 converges into lane 0 in the top half.
        let a = layout.push("A", &[]);
        assert_eq!((a.lane, a.color, a.width), (0, 0, 2));
        assert!(has(&a, line(0, 0, 0, LinePart::Top)));
        assert!(has(&a, line(1, 0, 1, LinePart::Top)));
        assert_eq!(layout.width(), 0);
    }

    #[test]
    fn two_tips_converge_on_one_parent() {
        let mut layout = GraphLayout::new();
        let x = layout.push("X", &ids(&["P"]));
        let y = layout.push("Y", &ids(&["P"]));
        assert_eq!((x.lane, y.lane), (0, 1));
        assert_ne!(x.color, y.color);
        // Y passes lane 0 through.
        assert!(has(&y, line(0, 0, 0, LinePart::Top)));
        assert!(has(&y, line(0, 0, 0, LinePart::Bottom)));
        let p = layout.push("P", &[]);
        assert_eq!((p.lane, p.width), (0, 2));
        assert!(has(&p, line(1, 0, 1, LinePart::Top)));
        assert!(has(&p, line(0, 0, 0, LinePart::Top)));
    }

    #[test]
    fn octopus_merge_opens_two_lanes() {
        let mut layout = GraphLayout::new();
        let o = layout.push("O", &ids(&["A", "B", "C"]));
        assert_eq!((o.lane, o.width), (0, 3));
        assert!(has(&o, line(0, 0, 0, LinePart::Bottom)));
        assert!(has(&o, line(0, 1, 1, LinePart::Bottom)));
        assert!(has(&o, line(0, 2, 2, LinePart::Bottom)));
        assert_eq!(layout.width(), 3);
    }

    #[test]
    fn second_parent_joins_an_existing_lane() {
        // Y waits for P in lane 0, then M merges P as its second parent: no third lane.
        let mut layout = GraphLayout::new();
        layout.push("Y", &ids(&["P"]));
        let m = layout.push("M", &ids(&["Q", "P"]));
        assert_eq!((m.lane, m.width), (1, 2));
        // The curve goes to lane 0 with that lane's color.
        assert!(has(&m, line(1, 0, 0, LinePart::Bottom)));
        assert_eq!(layout.width(), 2);
    }

    #[test]
    fn missing_parents_keep_their_lanes() {
        // The page ends before the parents show up: lanes wait.
        let mut layout = GraphLayout::new();
        layout.push("M", &ids(&["B", "C"]));
        layout.push("B", &ids(&["gone"]));
        assert_eq!(layout.width(), 2);
        // The next page continues the same layout.
        let c = layout.push("C", &ids(&["gone"]));
        assert_eq!(c.lane, 1);
        let gone = layout.push("gone", &[]);
        assert_eq!(gone.lane, 0);
        assert!(has(&gone, line(1, 0, 1, LinePart::Top)));
    }

    #[test]
    fn unknown_commit_takes_the_leftmost_free_lane() {
        let mut layout = GraphLayout::new();
        layout.push("A", &ids(&["a"]));
        layout.push("B", &ids(&["b"]));
        layout.push("C", &ids(&["c"]));
        // Lane 1 ends, then a new tip reuses it instead of growing to a fourth lane.
        layout.push("b", &[]);
        let d = layout.push("D", &ids(&["d"]));
        assert_eq!(d.lane, 1);
        assert_eq!(layout.width(), 3);
    }

    #[test]
    fn duplicate_parent_does_not_draw_a_self_curve() {
        let mut layout = GraphLayout::new();
        let m = layout.push("M", &ids(&["P", "P"]));
        assert_eq!(m.lane, 0);
        assert_eq!(m.lines, vec![line(0, 0, 0, LinePart::Bottom)]);
        assert_eq!(layout.width(), 1);
    }

    #[test]
    fn width_is_bounded_by_live_lanes() {
        // A busy history: several tips, merges and a root, checking the bound at every step.
        let history: Vec<(&str, Vec<&str>)> = vec![
            ("t1", vec!["a"]),
            ("t2", vec!["b"]),
            ("t3", vec!["a", "b", "c"]),
            ("a", vec!["d"]),
            ("b", vec!["d"]),
            ("c", vec!["d"]),
            ("t4", vec!["d", "x"]),
            ("d", vec!["e"]),
            ("e", vec![]),
        ];
        let mut layout = GraphLayout::new();
        for (oid, parents) in history {
            let parents = ids(&parents);
            let before = layout.width();
            let row = layout.push(oid, &parents);
            let after = layout.width();
            assert!(
                row.width <= before.max(after).max(row.lane + 1) + 1,
                "{oid}: width {} before {before} after {after}",
                row.width
            );
            // Every segment stays inside the row's width.
            for l in &row.lines {
                assert!(l.from < row.width && l.to < row.width, "{oid}: {l:?}");
            }
            assert!(row.lane < row.width);
        }
    }
}
