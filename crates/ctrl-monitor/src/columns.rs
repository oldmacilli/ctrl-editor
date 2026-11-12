//! Column widths of the monitor table, shared by header and rows
//! (docs/MONITOR.md §5). The unit is the front end's: pixels in Slint,
//! cells in the terminal. The direction column is fixed and the
//! description takes the remaining width; neither is resizable.

/// A resizable column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Time,
    Port,
    Channel,
    Message,
    Value,
}

impl Column {
    pub const ALL: [Column; 5] = [
        Column::Time,
        Column::Port,
        Column::Channel,
        Column::Message,
        Column::Value,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Column::Time => "time",
            Column::Port => "port",
            Column::Channel => "ch",
            Column::Message => "message",
            Column::Value => "value",
        }
    }

    /// The next column, wrapping; `step` may be negative.
    pub fn step(self, step: i32) -> Column {
        let n = Column::ALL.len() as i32;
        let i = Column::ALL.iter().position(|&c| c == self).unwrap() as i32;
        Column::ALL[(i + step).rem_euclid(n) as usize]
    }

    fn index(self) -> usize {
        Column::ALL.iter().position(|&c| c == self).unwrap()
    }
}

/// One width per [`Column`], never below its minimum. Header and rows read
/// the same value, so their cells line up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnWidths {
    widths: [u32; 5],
    mins: [u32; 5],
}

impl ColumnWidths {
    /// Defaults and minimums in [`Column::ALL`] order; a default below its
    /// minimum is raised.
    pub fn new(defaults: [u32; 5], mins: [u32; 5]) -> ColumnWidths {
        let mut widths = defaults;
        for (w, m) in widths.iter_mut().zip(mins) {
            *w = (*w).max(m);
        }
        ColumnWidths { widths, mins }
    }

    /// Terminal cells: `time` 9, `port` 12, `ch` 2, `message` 16, `value` 44.
    pub fn cells() -> ColumnWidths {
        ColumnWidths::new([9, 12, 2, 16, 44], [4, 4, 2, 7, 8])
    }

    /// Logical pixels, as the Slint pane starts.
    pub fn pixels() -> ColumnWidths {
        ColumnWidths::new([86, 104, 28, 156, 270], [48, 48, 24, 70, 70])
    }

    pub fn get(&self, c: Column) -> u32 {
        self.widths[c.index()]
    }

    pub fn min(&self, c: Column) -> u32 {
        self.mins[c.index()]
    }

    /// Sets a width, clamped to the minimum; returns what it became.
    pub fn set(&mut self, c: Column, width: u32) -> u32 {
        let w = width.max(self.mins[c.index()]);
        self.widths[c.index()] = w;
        w
    }

    /// Widens (positive) or narrows a column; returns the new width.
    pub fn resize(&mut self, c: Column, delta: i32) -> u32 {
        let w = (self.get(c) as i64 + delta as i64).clamp(0, u32::MAX as i64) as u32;
        self.set(c, w)
    }

    /// All widths in [`Column::ALL`] order, for a host that persists them.
    pub fn to_array(&self) -> [u32; 5] {
        self.widths
    }

    /// Restores saved widths, applying the minimums.
    pub fn from_array(&mut self, widths: [u32; 5]) {
        for (c, w) in Column::ALL.iter().zip(widths) {
            self.set(*c, w);
        }
    }
}

impl Default for ColumnWidths {
    fn default() -> Self {
        ColumnWidths::cells()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_clamps_to_the_minimum() {
        let mut w = ColumnWidths::cells();
        assert_eq!(w.resize(Column::Port, 3), 15);
        assert_eq!(w.resize(Column::Port, -100), w.min(Column::Port));
        assert_eq!(w.get(Column::Port), 4);
        assert_eq!(w.resize(Column::Channel, -1), 2);
    }

    #[test]
    fn defaults_below_the_minimum_are_raised_and_round_trip() {
        let w = ColumnWidths::new([1; 5], [3; 5]);
        assert_eq!(w.to_array(), [3; 5]);
        let mut p = ColumnWidths::pixels();
        p.from_array([10, 200, 10, 300, 10]);
        assert_eq!(p.to_array(), [48, 200, 24, 300, 70]);
    }

    #[test]
    fn focus_steps_wrap() {
        assert_eq!(Column::Value.step(1), Column::Time);
        assert_eq!(Column::Time.step(-1), Column::Value);
    }
}
