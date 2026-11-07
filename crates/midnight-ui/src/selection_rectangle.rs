//! Bounded presentation geometry. Entries and selection work remain on workers.
use midnight_core::{SelectionRanges, SelectionSpan};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Grid {
    pub mode: i32,
    pub columns: usize,
    pub page_rows: usize,
    /// Display rows, including the optional parent row.
    pub count: usize,
    pub parent: usize,
    pub width: f32,
    pub row_height: f32,
}
impl Grid {
    fn shape(self) -> Result<(usize, usize), &'static str> {
        if self.columns == 0
            || self.columns > SelectionRanges::MAX_SPANS
            || self.page_rows == 0
            || self.parent > 1
            || self.parent > self.count
            || !self.width.is_finite()
            || self.width <= 0.0
            || !self.row_height.is_finite()
            || self.row_height <= 0.0
        {
            return Err("Selection rectangle layout is unavailable");
        }
        let page = self
            .columns
            .checked_mul(self.page_rows)
            .ok_or("Selection rectangle layout is too large")?;
        let bands = if self.mode == 1 && self.count > 0 {
            let complete = (self.count - 1) / page;
            complete
                .checked_mul(self.page_rows)
                .and_then(|n| n.checked_add(self.page_rows.min(self.count - complete * page)))
                .ok_or("Selection rectangle layout is too large")?
        } else {
            self.count.div_ceil(self.columns)
        };
        Ok((page, bands))
    }

    pub fn empty_at(self, point: (f32, f32)) -> bool {
        let Ok((page, _)) = self.shape() else {
            return false;
        };
        if !point.0.is_finite()
            || !point.1.is_finite()
            || point.0 < 0.0
            || point.0 >= self.width
            || point.1 < 0.0
        {
            return false;
        }
        let column = (point.0 / self.width * self.columns as f32).floor() as usize;
        let band = (point.1 / self.row_height).floor() as usize;
        let row = if self.mode == 1 {
            (band / self.page_rows)
                .checked_mul(page)
                .and_then(|p| {
                    column
                        .checked_mul(self.page_rows)
                        .and_then(|c| p.checked_add(c))
                })
                .and_then(|p| p.checked_add(band % self.page_rows))
        } else {
            band.checked_mul(self.columns)
                .and_then(|p| p.checked_add(column))
        };
        row.is_none_or(|row| row >= self.count)
    }

    /// Coordinates are in the list's content space. Construction never walks entries.
    pub fn ranges(self, from: (f32, f32), to: (f32, f32)) -> Result<SelectionRanges, &'static str> {
        let (page_size, bands) = self.shape()?;
        if ![from.0, from.1, to.0, to.1].into_iter().all(f32::is_finite) {
            return Err("Selection rectangle position is unavailable");
        }
        let extent = self.count - self.parent;
        let mut output = Spans {
            values: Vec::new(),
            parent: self.parent,
            count: self.count,
        };
        let left = from.0.min(to.0).clamp(0.0, self.width);
        let right = from.0.max(to.0).clamp(0.0, self.width);
        let bottom = from.1.max(to.1).max(0.0);
        let top = from.1.min(to.1).max(0.0);
        if bands == 0 || left >= right || top >= bottom {
            return SelectionRanges::new(extent, vec![]);
        }
        let first_band = (top / self.row_height).floor() as usize;
        if first_band >= bands {
            return SelectionRanges::new(extent, vec![]);
        }
        let last_band = ((bottom / self.row_height).ceil() as usize)
            .saturating_sub(1)
            .min(bands - 1);
        let first_column =
            ((left / self.width * self.columns as f32).floor() as usize).min(self.columns - 1);
        let last_column = ((right / self.width * self.columns as f32).ceil() as usize)
            .saturating_sub(1)
            .min(self.columns - 1);
        let all_columns = first_column == 0 && last_column == self.columns - 1;
        if self.mode != 1 {
            if all_columns {
                output.push(
                    first_band * self.columns,
                    last_band
                        .saturating_add(1)
                        .saturating_mul(self.columns)
                        .saturating_sub(1),
                )?;
            } else {
                for band in first_band..=last_band {
                    output.push(
                        band * self.columns + first_column,
                        band * self.columns + last_column,
                    )?;
                }
            }
        } else {
            let last_page = last_band / self.page_rows;
            let mut page = first_band / self.page_rows;
            while page <= last_page {
                let start = if page == first_band / self.page_rows {
                    first_band % self.page_rows
                } else {
                    0
                };
                let end = if page == last_page {
                    last_band % self.page_rows
                } else {
                    self.page_rows - 1
                };
                if all_columns && start == 0 && end == self.page_rows - 1 {
                    let complete_end = if last_band % self.page_rows == self.page_rows - 1 {
                        last_page
                    } else {
                        last_page.saturating_sub(1)
                    };
                    output.push(
                        page * page_size,
                        complete_end
                            .saturating_add(1)
                            .saturating_mul(page_size)
                            .saturating_sub(1),
                    )?;
                    page = complete_end + 1;
                } else {
                    for column in first_column..=last_column {
                        let base = page * page_size + column * self.page_rows;
                        output.push(base + start, base + end)?;
                    }
                    page += 1;
                }
            }
        }
        SelectionRanges::new(extent, output.values)
    }
}

struct Spans {
    values: Vec<SelectionSpan>,
    parent: usize,
    count: usize,
}
impl Spans {
    fn push(&mut self, first: usize, last: usize) -> Result<(), &'static str> {
        let first = first.max(self.parent);
        let last = last.min(self.count.saturating_sub(1));
        if first > last {
            return Ok(());
        }
        let first = first - self.parent;
        let last = last - self.parent;
        if let Some(previous) = self.values.last_mut()
            && first <= previous.last.saturating_add(1)
        {
            previous.last = previous.last.max(last);
            return Ok(());
        }
        if self.values.len() == SelectionRanges::MAX_SPANS {
            return Err(
                "Selection rectangle exceeds the range budget; select fewer separated rows",
            );
        }
        self.values.push(SelectionSpan { first, last });
        Ok(())
    }
}

#[cfg(test)]
#[path = "selection_rectangle_tests.rs"]
mod tests;
