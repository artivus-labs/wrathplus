//! The plain `MessageFrame`'s state, the sibling of [`super::ScrollingMessageState`].

use std::collections::VecDeque;

use super::{quantize_fade, quantize_u8, MessageLine};

/// Where a MessageFrame's newest message enters: XML `insertMode` (`0x87a618`) and
/// `SetInsertMode`/`GetInsertMode` (`0x794ed0`/`0x794ff0`); the scrolling class has neither.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InsertMode {
    /// `"TOP"` (0): newest on the top line, older stepping down, as `UIErrorsFrame.xml:4` asks.
    Top,
    /// `"BOTTOM"` (1, the ctor default): newest on the bottom line, older stepping up.
    #[default]
    Bottom,
}

/// A `CSimpleMessageFrame`'s runtime state (ctor `0x785640`), the class of `UIErrorsFrame` and a
/// sibling of [`super::ScrollingMessageState`]: no ring, no `maxLines` (the cap is what fits), no
/// scrollback, an `AddMessage` alpha in place of the id, and a faded line freed (`0x786570`).
///
/// The reference queues `AddMessage` and drains it at OnUpdate, dropping a message while
/// `numLinesDisplayed` is 0 (`0x786265`); here a line lands at once and
/// [`Self::trim_to_viewport`] applies the cap.
#[derive(Clone, Debug, PartialEq)]
pub struct MessageFrameState {
    /// The frame's own `SetJustifyH`/`SetJustifyV` (method table `0x87b960`), `None` until called.
    /// Deviation: kept apart from the font's, as [`super::ScrollingMessageState::justify`] is.
    pub justify: Option<crate::justify::Justify>,

    /// The measure sweep's skip token, as [`super::ScrollingMessageState::lines_gen`].
    pub lines_gen: u64,
    /// The lines, newest at the back in both insert modes: [`InsertMode`] only orders the draw.
    pub lines: VecDeque<MessageLine>,
    /// `insertMode` (ctor BOTTOM).
    pub insert_mode: InsertMode,
    /// `timeVisible`/`displayDuration`: seconds a new line holds its insert alpha (ctor 10).
    pub time_visible: f32,
    /// `fadeDuration`: the fade's length in seconds (ctor 3); 0 hides the line at once.
    pub fade_duration: f32,
    /// `fadingEnabled` (ctor 1); while false only the vertical cap bounds the lines.
    pub fading_enabled: bool,
}

impl Default for MessageFrameState {
    fn default() -> MessageFrameState {
        // The ctor's defaults (`0x785640`).
        MessageFrameState {
            justify: None,
            lines: VecDeque::new(),
            insert_mode: InsertMode::default(),
            time_visible: 10.0,
            fade_duration: 3.0,
            fading_enabled: true,
            lines_gen: 0,
        }
    }
}

impl MessageFrameState {
    /// `AddMessage(text, r, g, b, a)` (`0x795590`, `0x785d00`): the fourth number is the line's
    /// starting alpha, where the scrolling class takes an id and forces opaque.
    pub fn add(&mut self, text: String, r: f32, g: f32, b: f32, a: f32) {
        self.lines_gen = self.lines_gen.wrapping_add(1);
        self.lines.push_back(MessageLine {
            text,
            id: 0,
            access_id: 0,
            extra_data: 0,
            color: [quantize_u8(r), quantize_u8(g), quantize_u8(b)],
            time_left: self.time_visible,
            fade_left: self.fade_duration,
            // Quantized like the colour: all four channels pack into one ARGB dword.
            alpha: f32::from(quantize_u8(a)) / 255.0,
            rows: 1,
            rows_key: 0,
        });
    }

    /// `Clear`: drop every line at once, no fade.
    pub fn clear(&mut self) {
        self.lines_gen = self.lines_gen.wrapping_add(1);
        self.lines.clear();
    }

    /// Advance the fade by `dt` (OnUpdate, `0x786200`) in the scrolling class's two phases, with no
    /// scroll gate, freeing a finished line (`0x786570`). The reference also gates on
    /// `numLinesDisplayed > 0`, which this does not check. The arithmetic is `f32`: whether
    /// `0x786200` keeps the unrounded x87 value like the scrolling tick is untraced.
    pub fn tick(&mut self, dt: f32) {
        if !self.fading_enabled {
            return;
        }
        for line in &mut self.lines {
            if line.time_left > 0.0 {
                line.time_left -= dt;
                continue;
            }
            if self.fade_duration <= 0.0 {
                line.alpha = 0.0;
                continue;
            }
            line.fade_left -= dt;
            if line.fade_left <= 0.0 {
                line.fade_left = 0.0;
                line.alpha = 0.0;
            } else {
                line.alpha = f32::from(quantize_fade(line.fade_left / self.fade_duration)) / 255.0;
            }
        }
        // A line past phase 1 at alpha 0 is finished, and freed.
        self.lines.retain(|l| l.time_left > 0.0 || l.alpha > 0.0);
    }

    /// Evict the oldest lines until the rest fit `viewport_rows`, each costing its wrapped rows.
    /// Zero rows means no rect yet and evicts nothing, so a message posted from `OnLoad` before
    /// the first layout survives; until then only the fade bounds the list.
    pub fn trim_to_viewport(&mut self, viewport_rows: usize) {
        if viewport_rows == 0 {
            return;
        }
        let mut used = 0usize;
        let mut keep = 0usize;
        for line in self.lines.iter().rev() {
            if used >= viewport_rows {
                break;
            }
            used += usize::from(line.rows.max(1));
            keep += 1;
        }
        while self.lines.len() > keep.max(1) {
            self.lines.pop_front();
        }
    }
}
