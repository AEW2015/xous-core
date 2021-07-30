
/*
  design ideas

Modal for password request:
    ---------------------
    | Password Type: Updater
    | Requester: RootKeys
    | Reason: The updater modal has not been set.
    | Security Level: Critical
    |
    |    *****4f_
    |
    |      ← 👁️ 🕶️ * →
    |--------------------

Item primitives:
  - text bubble
  - text entry field (with confidentiality option)
  - left/right radio select
  - up/down radio select

Then simple menu prompt after password entry:
    ---------------------
    | [x] Persist until reboot
    | [ ] Persist until suspend
    | [ ] Use once
    ---------------------

General form for modals:

    [top text]

    [action form]

    [bottom text]

 - "top text" is an optional TextArea
 - "action form" is a mandatory field that handles interactions
 - "bottom text" is an optional TextArea

 Action form can be exactly one of the following:
   - password text field - enter closes the form, has visibility options as left/right arrows; entered text wraps
   - regular text field - enter closes the form, visibility is always visible; entered text wraps
   - radio buttons - has an explicit "okay" button to close the modal; up/down arrows + select/enter pick the radio
   - check boxes - has an explicit "okay" button to close the modal; up/down arrows + select/enter checks boxes
   - slider - left/right moves the slider, enter/select closes the modal
*/
use enum_dispatch::enum_dispatch;
use xous::MessageEnvelope;
use xous::send_message;

use crate::api::*;
use crate::Gam;

use graphics_server::api::*;
pub use graphics_server::GlyphStyle;

use xous_ipc::{String, Buffer};
use num_traits::*;

use crate::MsgForwarder;

use core::fmt::Write;
use locales::t;

pub const MAX_ITEMS: usize = 8;

#[derive(Debug, Copy, Clone, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ItemName(String::<64>);
impl ItemName {
    pub fn new(name: &str) -> Self {
        ItemName(String::<64>::from_str(name))
    }
    pub fn as_str(&self) -> &str {
        self.0.as_str().expect("couldn't convert item into string")
    }
}
#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Copy, Clone, Eq, PartialEq)]
pub struct TextEntryPayload(pub String::<256>);
impl TextEntryPayload {
    pub fn new() -> Self {
        TextEntryPayload(String::<256>::new())
    }
    pub fn volatile_clear(&mut self) {
        self.0.volatile_clear(); // volatile_clear() ensures that 0's are written and not optimized out; important for password fields
    }
    pub fn as_str(&self) -> &str {
        self.0.as_str().expect("couldn't convert textentry string")
    }
}

#[derive(Debug, Copy, Clone, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct RadioButtonPayload(ItemName); // returns the name of the item corresponding to the radio button selection
impl RadioButtonPayload {
    pub fn new(name: &str) -> Self {
        RadioButtonPayload(ItemName::new(name))
    }
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}
#[derive(Debug, Copy, Clone, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct CheckBoxPayload([Option<ItemName>; MAX_ITEMS]); // returns a list of potential items that could be selected
impl CheckBoxPayload {
    pub fn new() -> Self {
        CheckBoxPayload([None; MAX_ITEMS])
    }
    pub fn payload(&self) -> [Option<ItemName>; MAX_ITEMS] {
        self.0
    }
    pub fn contains(&self, name: &str) -> bool {
        for maybe_item in self.0.iter() {
            if let Some(item) = maybe_item {
                if item.as_str() == name {
                    return true;
                }
            }
        }
        false
    }
    pub fn add(&mut self, name: &str) -> bool {
        if self.contains(name) {
            return true
        }
        for maybe_item in self.0.iter_mut() {
            if maybe_item.is_none() {
                *maybe_item = Some(ItemName::new(name));
                return true;
            }
        }
        false
    }
    pub fn remove(&mut self, name: &str) -> bool {
        for maybe_item in self.0.iter_mut() {
            if let Some(item) = maybe_item {
                if item.as_str() == name {
                    *maybe_item = None;
                    return true;
                }
            }
        }
        false
    }
}

#[derive(Debug, Copy, Clone, num_derive::FromPrimitive, num_derive::ToPrimitive)]
pub enum TextEntryVisibility {
    /// text is fully visible
    Visible = 0,
    /// only last chars are shown of text entry, the rest obscured with *
    LastChars = 1,
    /// all chars hidden as *
    Hidden = 2,
}
#[derive(Copy, Clone)]
pub struct TextEntry {
    pub is_password: bool,
    pub visibility: TextEntryVisibility,
    pub action_conn: xous::CID,
    pub action_opcode: u32,
    pub action_payload: TextEntryPayload,
    // validator borrows the text entry payload, and returns an error message if something didn't go well.
    // validator takes as ragument the current action_payload, and the current action_opcode
    pub validator: Option<fn(TextEntryPayload, u32) -> Option<xous_ipc::String::<512>> >,
}
impl ActionApi for TextEntry {
    fn set_action_opcode(&mut self, op: u32) {self.action_opcode = op}
    fn is_password(&self) -> bool {
        self.is_password
    }
    /// The total canvas height is computed with this API call
    /// The canvas height is not dynamically adjustable for modals.
    fn height(&self, glyph_height: i16, margin: i16) -> i16 {
        /*
            -------------------
            | ****            |    <-- glyph_height + 2*margin
            -------------------
                ← 👁️ 🕶️ * →        <-- glyph_height

            + 2 * margin top/bottom

            auto-closes on enter
        */
        if self.is_password {
            glyph_height + 2*margin + glyph_height + 2*margin + 8 // 8 pixels extra margin because the emoji glyphs are oversized
        } else {
            glyph_height + 2*margin
        }
    }
    fn redraw(&self, at_height: i16, modal: &Modal) {
        let color = if self.is_password {
            PixelColor::Light
        } else {
            PixelColor::Dark
        };

        // draw the currently entered text
        let mut tv = TextView::new(
            modal.canvas,
            TextBounds::BoundingBox(Rectangle::new(
                Point::new(modal.margin, at_height),
                Point::new(modal.canvas_width - modal.margin, at_height + modal.line_height))
        ));
        tv.ellipsis = true; // TODO: fix so we are drawing from the right-most entered text and old text is ellipsis *to the left*
        tv.invert = self.is_password;
        tv.style = modal.style;
        tv.margin = Point::new(0, 0);
        tv.draw_border = false;
        tv.insertion = Some(self.action_payload.0.len() as i32);
        tv.text.clear(); // make sure this is blank
        let payload_chars = self.action_payload.0.as_str().unwrap().chars().count();
        // TODO: condense the "above 20" chars length path a bit -- written out "the dumb way" just to reason out the logic a bit
        match self.visibility {
            TextEntryVisibility::Visible => {
                log::trace!("action payload: {}", self.action_payload.0.as_str().unwrap());
                if payload_chars < 20 {
                    write!(tv.text, "{}", self.action_payload.0.as_str().unwrap()).unwrap();
                } else {
                    write!(tv.text, "...{}", &self.action_payload.0.as_str().unwrap()[payload_chars-18..]).unwrap();
                }
                modal.gam.post_textview(&mut tv).expect("couldn't post textview");
            },
            TextEntryVisibility::Hidden => {
                if payload_chars < 20 {
                    for _char in self.action_payload.0.as_str().unwrap().chars() {
                        tv.text.push('*').expect("text field too long");
                    }
                } else {
                    // just render a pure dummy string
                    tv.text.push('.').unwrap();
                    tv.text.push('.').unwrap();
                    tv.text.push('.').unwrap();
                    for _ in 0..18 {
                        tv.text.push('*').expect("text field too long");
                    }
                }
                modal.gam.post_textview(&mut tv).expect("couldn't post textview");
            },
            TextEntryVisibility::LastChars => {
                if payload_chars < 20 {
                    let hide_to = if self.action_payload.0.as_str().unwrap().chars().count() >= 2 {
                        self.action_payload.0.as_str().unwrap().chars().count() - 2
                    } else {
                        0
                    };
                    for (index, ch) in self.action_payload.0.as_str().unwrap().chars().enumerate() {
                        if index < hide_to {
                            tv.text.push('*').expect("text field too long");
                        } else {
                            tv.text.push(ch).expect("text field too long");
                        }
                    }
                } else {
                    tv.text.push('.').unwrap();
                    tv.text.push('.').unwrap();
                    tv.text.push('.').unwrap();
                    let hide_to = if self.action_payload.0.as_str().unwrap().chars().count() >= 2 {
                        self.action_payload.0.as_str().unwrap().chars().count() - 2
                    } else {
                        0
                    };
                    for (index, ch) in self.action_payload.0.as_str().unwrap()[payload_chars-18..].chars().enumerate() {
                        if index + payload_chars-18 < hide_to {
                            tv.text.push('*').expect("text field too long");
                        } else {
                            tv.text.push(ch).expect("text field too long");
                        }
                    }
                }
                modal.gam.post_textview(&mut tv).expect("couldn't post textview");
            }
        }
        if self.is_password {
            // draw the visibility selection area
            // "<👀🤫✴️>" coded explicitly. Pasting unicode into vscode yields extra cruft that we can't parse (e.g. skin tones and color mods).
            let prompt = "\u{2b05} \u{1f440}\u{1f576}\u{26d4} \u{27a1}";
            let select_index = match self.visibility {
                TextEntryVisibility::Visible => 2,
                TextEntryVisibility::LastChars => 3,
                TextEntryVisibility::Hidden => 4,
            };
            let spacing = 38; // fixed width spacing for the array
            let emoji_width = 36;
            // center the prompt nicely, if possible
            let left_edge = if modal.canvas_width > prompt.chars().count() as i16 * spacing {
                (modal.canvas_width - prompt.chars().count() as i16 * spacing) / 2
            } else {
                0
            };
            for (i, ch) in prompt.chars().enumerate() {
                let mut tv = TextView::new(
                    modal.canvas,
                    TextBounds::BoundingBox(Rectangle::new(
                        Point::new(left_edge + i as i16 * spacing, at_height + modal.line_height + modal.margin * 4),
                        Point::new(left_edge + i as i16 * spacing + emoji_width, at_height + modal.line_height + 34 + modal.margin * 4))
                ));
                tv.style = GlyphStyle::Regular;
                tv.margin = Point::new(0, 0);
                tv.draw_border = false;
                if i == select_index {
                    tv.invert = !self.is_password;
                } else {
                    tv.invert = self.is_password;
                }
                tv.text.clear();
                write!(tv.text, "{}", ch).unwrap();
                log::trace!("tv.text: {} : {}/{}", i, tv.text, ch);
                modal.gam.post_textview(&mut tv).expect("couldn't post textview");
            }
        }

        // draw a line for where text gets entered (don't use a box, fitting could be awkward)
        modal.gam.draw_line(modal.canvas, Line::new_with_style(
            Point::new(modal.margin, at_height + modal.line_height + 4),
            Point::new(modal.canvas_width - modal.margin, at_height + modal.line_height + 4),
            DrawStyle::new(color, color, 1))
            ).expect("couldn't draw entry line");
    }
    fn key_action(&mut self, k: char) -> (Option<xous_ipc::String::<512>>, bool) {
        log::trace!("key_action: {}", k);
        match k {
            '←' => {
                if self.visibility as u32 > 0 {
                    match FromPrimitive::from_u32(self.visibility as u32 - 1) {
                        Some(new_visibility) => {
                            log::trace!("new visibility: {:?}", new_visibility);
                            self.visibility = new_visibility;
                        },
                        _ => {
                            panic!("internal error: an TextEntryVisibility did not resolve correctly");
                        }
                    }
                }
            },
            '→' => {
                if (self.visibility as u32) < (TextEntryVisibility::Hidden as u32) {
                    match FromPrimitive::from_u32(self.visibility as u32 + 1) {
                        Some(new_visibility) => {
                            log::trace!("new visibility: {:?}", new_visibility);
                            self.visibility = new_visibility
                        },
                        _ => {
                            panic!("internal error: an TextEntryVisibility did not resolve correctly");
                        }
                    }
                }
            },
            '∴' | '\u{d}' => {
                if let Some(validator) = self.validator {
                    if let Some(err_msg) = validator(self.action_payload, self.action_opcode) {
                        self.action_payload.0.clear(); // reset the input field
                        return (Some(err_msg), false);
                    }
                }

                let buf = Buffer::into_buf(self.action_payload).expect("couldn't convert message to payload");
                buf.send(self.action_conn, self.action_opcode).map(|_| ()).expect("couldn't send action message");
                self.action_payload.volatile_clear(); // ensure the local copy of text is zero'd out
                return (None, true)
            }
            '↑' | '↓' => {
                // ignore these navigation keys
            }
            '\u{0}' => {
                // ignore null messages
            }
            '\u{8}' => { // backspace
                // coded in a conservative manner to avoid temporary allocations that can leave the plaintext on the stack
                let mut temp_str = String::<256>::from_str(self.action_payload.0.as_str().unwrap());
                let cur_len = temp_str.as_str().unwrap().chars().count();
                let mut c_iter = temp_str.as_str().unwrap().chars();
                self.action_payload.0.clear();
                for _ in 0..cur_len-1 {
                    self.action_payload.0.push(c_iter.next().unwrap()).unwrap();
                }
                temp_str.volatile_clear();
            }
            _ => { // text entry
                self.action_payload.0.push(k).expect("ran out of space storing password");
                log::trace!("****update payload: {}", self.action_payload.0);
            }
        }
        (None, false)
    }
}
#[derive(Debug, Copy, Clone)]
pub struct RadioButtons {
    pub items: [Option<ItemName>; MAX_ITEMS],
    pub action_conn: xous::CID,
    pub action_opcode: u32,
    pub action_payload: RadioButtonPayload, // the current "radio button" selection
    pub select_index: i16, // the current candidate to be selected
    pub max_items: i16,
}
impl RadioButtons {
    pub fn new(action_conn: xous::CID, action_opcode: u32) -> Self {
        RadioButtons {
            items: [None; MAX_ITEMS],
            action_conn,
            action_opcode,
            action_payload: RadioButtonPayload::new(""),
            select_index: 0,
            max_items: 0,
        }
    }
    pub fn add_item(&mut self, new_item: ItemName) -> Option<ItemName> {
        if self.action_payload.as_str().len() == 0 {
            // default to the first item added
            self.action_payload = RadioButtonPayload::new(new_item.as_str());
        }
        for item in self.items.iter_mut() {
            if item.is_none() {
                self.max_items += 1;
                *item = Some(new_item);
                return None;
            }
        }
        return Some(new_item);
    }
}
impl ActionApi for RadioButtons {
    fn set_action_opcode(&mut self, op: u32) {self.action_opcode = op}
    fn height(&self, glyph_height: i16, margin: i16) -> i16 {
        let mut total_items = 0;
        // total items, then +1 for the "Okay" message
        for item in self.items.iter() {
            if item.is_some(){ total_items += 1}
        }
        (total_items + 1) * glyph_height + margin * 2 + 5 // +4 for some bottom margin slop
    }
    fn redraw(&self, at_height: i16, modal: &Modal) {
        // prime a textview with the correct general style parameters
        let mut tv = TextView::new(
            modal.canvas,
            TextBounds::BoundingBox(Rectangle::new_coords(0, 0, 1, 1))
        );
        tv.ellipsis = true;
        tv.style = modal.style;
        tv.invert = false;
        tv.draw_border= false;
        tv.margin = Point::new(0, 0,);
        tv.insertion = None;

        let cursor_x = modal.margin;
        let select_x = modal.margin + 20;
        let text_x = modal.margin + 20 + 20;

        //let mut emoji_slop = (36 - modal.line_height) / 2;
        //if emoji_slop < 0 { emoji_slop = 0; }
        let emoji_slop = 2; // tweaked for a non-emoji glyph

        let mut cur_line = 0;
        let mut do_okay = true;
        for maybe_item in self.items.iter() {
            if let Some(item) = maybe_item {
                let cur_y = at_height + cur_line * modal.line_height;
                if cur_line == self.select_index {
                    // draw the cursor
                    tv.text.clear();
                    tv.bounds_computed = None;
                    tv.bounds_hint = TextBounds::BoundingBox(Rectangle::new(
                        Point::new(cursor_x, cur_y - emoji_slop), Point::new(cursor_x + 36, cur_y - emoji_slop + 36)
                    ));
                    write!(tv, "»").unwrap();
                    modal.gam.post_textview(&mut tv).expect("couldn't post tv");
                    do_okay = false;
                }
                if item.as_str() == self.action_payload.as_str() {
                    // draw the radio dot
                    tv.text.clear();
                    tv.bounds_computed = None;
                    tv.bounds_hint = TextBounds::BoundingBox(Rectangle::new(
                        Point::new(select_x, cur_y), Point::new(select_x + 36, cur_y + modal.line_height)
                    ));
                    write!(tv, "•").unwrap();
                    modal.gam.post_textview(&mut tv).expect("couldn't post tv");
                }
                // draw the text
                tv.text.clear();
                tv.bounds_computed = None;
                tv.bounds_hint = TextBounds::BoundingBox(Rectangle::new(
                    Point::new(text_x, cur_y), Point::new(modal.canvas_width - modal.margin, cur_y + modal.line_height)
                ));
                write!(tv, "{}", item.as_str()).unwrap();
                modal.gam.post_textview(&mut tv).expect("couldn't post tv");

                cur_line += 1;
            }
        }
        cur_line += 1;
        let cur_y = at_height + cur_line * modal.line_height;
        if do_okay {
            tv.text.clear();
            tv.bounds_computed = None;
            tv.bounds_hint = TextBounds::BoundingBox(Rectangle::new(
                Point::new(cursor_x, cur_y - emoji_slop), Point::new(cursor_x + 36, cur_y - emoji_slop + 36)
            ));
            write!(tv, "»").unwrap(); // right arrow emoji. use unicode numbers, because text editors do funny shit with emojis
            modal.gam.post_textview(&mut tv).expect("couldn't post tv");
        }
        // draw the "OK" line
        tv.text.clear();
        tv.bounds_computed = None;
        tv.bounds_hint = TextBounds::BoundingBox(Rectangle::new(
            Point::new(text_x, cur_y), Point::new(modal.canvas_width - modal.margin, cur_y + modal.line_height)
        ));
        write!(tv, "{}", t!("radio.select_and_close", xous::LANG)).unwrap();
        modal.gam.post_textview(&mut tv).expect("couldn't post tv");

        // divider lines
        modal.gam.draw_line(modal.canvas, Line::new_with_style(
            Point::new(modal.margin, at_height),
            Point::new(modal.canvas_width - modal.margin, at_height),
            DrawStyle::new(PixelColor::Dark, PixelColor::Dark, 1))
            ).expect("couldn't draw entry line");
    }
    fn key_action(&mut self, k: char) -> (Option<xous_ipc::String::<512>>, bool) {
        log::trace!("key_action: {}", k);
        match k {
            '←' | '→' => {
                // ignore these navigation keys
            },
            '↑' => {
                if self.select_index > 0 {
                    self.select_index -= 1;
                }
            }
            '↓' => {
                if self.select_index < self.max_items + 1 { // +1 is the "OK" button
                    self.select_index += 1;
                }
            }
            '∴' | '\u{d}' => {
                if self.select_index < self.max_items {
                    // iterate through to find the index -- because if we support a remove() API later,
                    // the list can have "holes", such that the index != index in the array
                    let mut cur_index = 0;
                    for maybe_item in self.items.iter() {
                        if let Some(item) = maybe_item {
                            if cur_index == self.select_index {
                                self.action_payload = RadioButtonPayload::new(item.as_str());
                                break;
                            }
                            cur_index += 1;
                        }
                    }
                } else {  // the OK button select
                    let buf = Buffer::into_buf(self.action_payload).expect("couldn't convert message to payload");
                    buf.send(self.action_conn, self.action_opcode).map(|_| ()).expect("couldn't send action message");
                    return (None, true)
                }
            }
            '\u{0}' => {
                // ignore null messages
            }
            _ => {
                // ignore text entry
            }
        }
        (None, false)
    }
}
#[derive(Debug, Copy, Clone)]
pub struct CheckBoxes {
    pub items: [Option<ItemName>; MAX_ITEMS],
    pub action_conn: xous::CID,
    pub action_opcode: u32,
    pub action_payload: CheckBoxPayload,
    pub max_items: i16,
    pub select_index: i16,
}
impl CheckBoxes {
    pub fn new(action_conn: xous::CID, action_opcode: u32) -> Self {
        CheckBoxes {
            items: [None; MAX_ITEMS],
            action_conn,
            action_opcode,
            action_payload: CheckBoxPayload::new(),
            max_items: 0,
            select_index: 0,
        }
    }
    pub fn add_item(&mut self, new_item: ItemName) -> Option<ItemName> {
        for item in self.items.iter_mut() {
            if item.is_none() {
                self.max_items += 1;
                *item = Some(new_item);
                return None;
            }
        }
        return Some(new_item);
    }
}
impl ActionApi for CheckBoxes {
    fn set_action_opcode(&mut self, op: u32) {self.action_opcode = op}
    fn height(&self, glyph_height: i16, margin: i16) -> i16 {
        let mut total_items = 0;
        // total items, then +1 for the "Okay" message
        for item in self.items.iter() {
            if item.is_some(){ total_items += 1}
        }
        (total_items + 1) * glyph_height + margin * 2 + 5 // some slop needed because of the prompt character
    }
    fn redraw(&self, at_height: i16, modal: &Modal) {
        // prime a textview with the correct general style parameters
        let mut tv = TextView::new(
            modal.canvas,
            TextBounds::BoundingBox(Rectangle::new_coords(0, 0, 1, 1))
        );
        tv.ellipsis = true;
        tv.style = modal.style;
        tv.invert = false;
        tv.draw_border= false;
        tv.margin = Point::new(0, 0,);
        tv.insertion = None;

        let cursor_x = modal.margin;
        let select_x = modal.margin + 20;
        let text_x = modal.margin + 20 + 20;

        let emoji_slop = 2; // tweaked for a non-emoji glyph

        let mut cur_line = 0;
        let mut do_okay = true;
        for maybe_item in self.items.iter() {
            if let Some(item) = maybe_item {
                let cur_y = at_height + cur_line * modal.line_height;
                if cur_line == self.select_index {
                    // draw the cursor
                    tv.text.clear();
                    tv.bounds_computed = None;
                    tv.bounds_hint = TextBounds::BoundingBox(Rectangle::new(
                        Point::new(cursor_x, cur_y - emoji_slop), Point::new(cursor_x + 36, cur_y - emoji_slop + 36)
                    ));
                    write!(tv, "»").unwrap();
                    modal.gam.post_textview(&mut tv).expect("couldn't post tv");
                    do_okay = false;
                }
                if self.action_payload.contains(item.as_str()) {
                    // draw the check mark
                    tv.text.clear();
                    tv.bounds_computed = None;
                    tv.bounds_hint = TextBounds::BoundingBox(Rectangle::new(
                        Point::new(select_x, cur_y - emoji_slop), Point::new(select_x + 36, cur_y + modal.line_height)
                    ));
                    write!(tv, "\u{d7}").unwrap(); // multiplication sign
                    modal.gam.post_textview(&mut tv).expect("couldn't post tv");
                }
                // draw the text
                tv.text.clear();
                tv.bounds_computed = None;
                tv.bounds_hint = TextBounds::BoundingBox(Rectangle::new(
                    Point::new(text_x, cur_y), Point::new(modal.canvas_width - modal.margin, cur_y + modal.line_height)
                ));
                write!(tv, "{}", item.as_str()).unwrap();
                modal.gam.post_textview(&mut tv).expect("couldn't post tv");

                cur_line += 1;
            }
        }
        cur_line += 1;
        let cur_y = at_height + cur_line * modal.line_height;
        if do_okay {
            tv.text.clear();
            tv.bounds_computed = None;
            tv.bounds_hint = TextBounds::BoundingBox(Rectangle::new(
                Point::new(cursor_x, cur_y - emoji_slop), Point::new(cursor_x + 36, cur_y - emoji_slop + 36)
            ));
            write!(tv, "»").unwrap(); // right arrow emoji. use unicode numbers, because text editors do funny shit with emojis
            modal.gam.post_textview(&mut tv).expect("couldn't post tv");
        }
        // draw the "OK" line
        tv.text.clear();
        tv.bounds_computed = None;
        tv.bounds_hint = TextBounds::BoundingBox(Rectangle::new(
            Point::new(text_x, cur_y), Point::new(modal.canvas_width - modal.margin, cur_y + modal.line_height)
        ));
        write!(tv, "{}", t!("radio.select_and_close", xous::LANG)).unwrap();
        modal.gam.post_textview(&mut tv).expect("couldn't post tv");

        // divider lines
        modal.gam.draw_line(modal.canvas, Line::new_with_style(
            Point::new(modal.margin, at_height),
            Point::new(modal.canvas_width - modal.margin, at_height),
            DrawStyle::new(PixelColor::Dark, PixelColor::Dark, 1))
            ).expect("couldn't draw entry line");
    }
    fn key_action(&mut self, k: char) -> (Option<xous_ipc::String::<512>>, bool) {
        log::trace!("key_action: {}", k);
        match k {
            '←' | '→' => {
                // ignore these navigation keys
            },
            '↑' => {
                if self.select_index > 0 {
                    self.select_index -= 1;
                }
            }
            '↓' => {
                if self.select_index < self.max_items + 1 { // +1 is the "OK" button
                    self.select_index += 1;
                }
            }
            '∴' | '\u{d}' => {
                if self.select_index < self.max_items {
                    // iterate through to find the index -- because if we support a remove() API later,
                    // the list can have "holes", such that the index != index in the array
                    let mut cur_index = 0;
                    for maybe_item in self.items.iter() {
                        if let Some(item) = maybe_item {
                            if cur_index == self.select_index {
                                if self.action_payload.contains(item.as_str()) {
                                    self.action_payload.remove(item.as_str());
                                } else {
                                    self.action_payload.add(item.as_str());
                                }
                                break;
                            }
                            cur_index += 1;
                        }
                    }
                } else {  // the OK button select
                    let buf = Buffer::into_buf(self.action_payload).expect("couldn't convert message to payload");
                    buf.send(self.action_conn, self.action_opcode).map(|_| ()).expect("couldn't send action message");
                    return (None, true)
                }
            }
            '\u{0}' => {
                // ignore null messages
            }
            _ => {
                // ignore text entry
            }
        }
        (None, false)
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Notification {
    pub action_conn: xous::CID,
    pub action_opcode: u32,
    pub is_password: bool,
}
impl Notification {
    pub fn new(action_conn: xous::CID, action_opcode: u32) -> Self {
        Notification {
            action_conn,
            action_opcode,
            is_password: false,
        }
    }
    pub fn set_is_password(&mut self, setting: bool) {
        // this will cause text to be inverted. Untrusted entities can try to set this,
        // but the GAM should defeat this for dialog boxes outside of the trusted boot
        // set because they can't achieve a high enough trust level.
        self.is_password = true;
    }
}
impl ActionApi for Notification {
    fn set_action_opcode(&mut self, op: u32) {self.action_opcode = op}
    fn height(&self, glyph_height: i16, margin: i16) -> i16 {
        glyph_height + margin * 2 + 5
    }
    fn redraw(&self, at_height: i16, modal: &Modal) {
        // prime a textview with the correct general style parameters
        let mut tv = TextView::new(
            modal.canvas,
            TextBounds::BoundingBox(Rectangle::new_coords(0, 0, 1, 1))
        );
        tv.ellipsis = true;
        tv.style = modal.style;
        tv.invert = self.is_password;
        tv.draw_border= false;
        tv.margin = Point::new(0, 0,);
        tv.insertion = None;

        tv.bounds_computed = None;
        tv.bounds_hint = TextBounds::GrowableFromTl(
            Point::new(modal.margin, at_height + modal.margin * 2),
            (modal.canvas_width - modal.margin * 2) as u16
        );
        write!(tv, "{}", t!("notification.dismiss", xous::LANG)).unwrap();
        modal.gam.bounds_compute_textview(&mut tv).expect("couldn't simulate text size");
        let textwidth = if let Some(bounds) = tv.bounds_computed {
            bounds.br.x - bounds.tl.x
        } else {
            modal.canvas_width - modal.margin * 2
        };
        log::info!("tw: {}", textwidth);
        let offset = (modal.canvas_width - textwidth) / 2;
        log::info!("offset2: {}", offset);
        tv.bounds_computed = None;
        tv.bounds_hint = TextBounds::BoundingBox(Rectangle::new(
            Point::new(offset, at_height + modal.margin * 2),
            Point::new(modal.canvas_width - modal.margin, at_height + modal.line_height + modal.margin * 2)
        ));
        modal.gam.post_textview(&mut tv).expect("couldn't post tv");

        // divider lines
        let color = if self.is_password {
            PixelColor::Light
        } else {
            PixelColor::Dark
        };

        modal.gam.draw_line(modal.canvas, Line::new_with_style(
            Point::new(modal.margin, at_height + modal.margin),
            Point::new(modal.canvas_width - modal.margin, at_height + modal.margin),
            DrawStyle::new(color, color, 1))
            ).expect("couldn't draw entry line");
    }
    fn key_action(&mut self, k: char) -> (Option<xous_ipc::String::<512>>, bool) {
        log::trace!("key_action: {}", k);
        match k {
            '\u{0}' => {
                // ignore null messages
            }
            _ => {
                send_message(self.action_conn, xous::Message::new_scalar(self.action_opcode as usize, 0, 0, 0, 0)).expect("couldn't pass on dismissal");
                return(None, true)
            }
        }
        (None, false)
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Slider {
    pub min: u32,
    pub max: u32,
    pub step: u32,
    pub action_conn: xous::CID,
    pub action_opcode: u32,
    pub action_payload: u32,
}
impl ActionApi for Slider {
    fn height(&self, glyph_height: i16, margin: i16) -> i16 {
        /*
            min            max    <- glyph height
             -----O----------     <- glyph height
                 [ Okay ]         <- glyph height
        */
        glyph_height * 3 + margin * 2
    }
    fn set_action_opcode(&mut self, op: u32) {self.action_opcode = op}
}





#[enum_dispatch]
pub trait ActionApi {
    fn height(&self, glyph_height: i16, margin: i16) -> i16 {glyph_height + margin * 2}
    fn redraw(&self, _at_height: i16, _modal: &Modal) { unimplemented!() }
    fn close(&mut self) {}
    fn is_password(&self) -> bool { false }
    /// navigation is one of '∴' | '←' | '→' | '↑' | '↓'
    fn key_action(&mut self, _key: char) -> (Option<xous_ipc::String::<512>>, bool) {(None, true)}
    fn set_action_opcode(&mut self, _op: u32) {}
}

#[enum_dispatch(ActionApi)]
#[derive(Copy, Clone)]
pub enum ActionType {
    TextEntry,
    RadioButtons,
    CheckBoxes,
    Slider,
    Notification,
}

//#[derive(Debug)]
pub struct Modal<'a> {
    pub sid: xous::SID,
    pub gam: Gam,
    pub xns: xous_names::XousNames,
    pub top_text: Option<TextView>,
    pub bot_text: Option<TextView>,
    pub action: ActionType,

    //pub index: usize, // currently selected item
    pub canvas: Gid,
    pub authtoken: [u32; 4],
    pub margin: i16,
    pub line_height: i16,
    pub canvas_width: i16,
    pub inverted: bool,
    pub style: GlyphStyle,
    pub helper_data: Option<Buffer<'a>>,
    pub name: String::<128>,
}

#[derive(Debug, num_derive::FromPrimitive, num_derive::ToPrimitive)]
pub enum ModalOpcode { // if changes are made here, also update MenuOpcode
    Redraw = 0x4000_0000, // set the high bit so that "standard" enums don't conflict with the Modal-specific opcodes
    Rawkeys,
    Quit,
}

fn recompute_canvas(modal: &mut Modal, action: ActionType, top_text: Option<&str>, bot_text: Option<&str>, style: GlyphStyle) {
    // we need to set a "max" size to our modal box, so that the text computations don't fail later on
    let current_bounds = modal.gam.get_canvas_bounds(modal.canvas).expect("couldn't get current bounds");
    let mut new_bounds = SetCanvasBoundsRequest {
        requested: Point::new(current_bounds.x, crate::api::MODAL_Y_MAX),
        granted: None,
        token_type: TokenType::App,
        token: modal.authtoken,
    };
    log::debug!("applying recomputed bounds of {:?}", new_bounds);
    modal.gam.set_canvas_bounds_request(&mut new_bounds).expect("couldn't call set bounds");

    // method:
    //   - we assume the GAM gives us an initial modal with a "maximum" height setting
    //   - items are populated within this maximal canvas setting, and then the actual height needed is computed
    //   - the canvas is resized to this actual height
    // problems:
    //   - there is no sanity check on the size of the text boxes. So if you give the UX element a top_text box that's
    //     huge, it will just overflow the canvas size and nothing else will get drawn.

    let mut total_height = modal.margin;
    log::trace!("step 0 total_height: {}", total_height);
    // compute height of top_text, if any
    if let Some(top_str) = top_text {
        let mut top_tv = TextView::new(modal.canvas,
            TextBounds::GrowableFromTl(
                Point::new(modal.margin, total_height),
                (modal.canvas_width - modal.margin * 2) as u16
            ));
        top_tv.draw_border = false;
        top_tv.style = style;
        top_tv.margin = Point::new(0, 0,); // all margin already accounted for in the raw bounds of the text drawing
        top_tv.ellipsis = false;
        top_tv.invert = modal.inverted;
        write!(top_tv.text, "{}", top_str).unwrap();

        log::trace!("posting top tv: {:?}", top_tv);
        modal.gam.bounds_compute_textview(&mut top_tv).expect("couldn't simulate top text size");
        if let Some(bounds) = top_tv.bounds_computed {
            total_height += bounds.br.y - bounds.tl.y;
        } else {
            log::error!("couldn't compute height for modal top_text: {:?}", top_tv);
            panic!("couldn't compute height for modal top_text");
        }
        modal.top_text = Some(top_tv);
    }
    total_height += modal.margin;

    // compute height of action item
    log::trace!("step 1 total_height: {}", total_height);
    total_height += action.height(modal.line_height, modal.margin);
    total_height += modal.margin;

    // compute height of bot_text, if any
    log::trace!("step 2 total_height: {}", total_height);
    if let Some(bot_str) = bot_text {
        let mut bot_tv = TextView::new(modal.canvas,
            TextBounds::GrowableFromTl(
                Point::new(modal.margin, total_height),
                (modal.canvas_width - modal.margin * 2) as u16
            ));
        bot_tv.draw_border = false;
        bot_tv.style = style;
        bot_tv.margin = Point::new(0, 0,); // all margin already accounted for in the raw bounds of the text drawing
        bot_tv.ellipsis = false;
        bot_tv.invert = modal.inverted;
        write!(bot_tv.text, "{}", bot_str).unwrap();

        log::trace!("posting bot tv: {:?}", bot_tv);
        modal.gam.bounds_compute_textview(&mut bot_tv).expect("couldn't simulate bot text size");
        if let Some(bounds) = bot_tv.bounds_computed {
            total_height += bounds.br.y - bounds.tl.y;
        } else {
            log::error!("couldn't compute height for modal bot_text: {:?}", bot_tv);
            panic!("couldn't compute height for modal bot_text");
        }
        modal.bot_text = Some(bot_tv);
        total_height += modal.margin;
    }
    log::trace!("step 3 total_height: {}", total_height);

    let current_bounds = modal.gam.get_canvas_bounds(modal.canvas).expect("couldn't get current bounds");
    let mut new_bounds = SetCanvasBoundsRequest {
        requested: Point::new(current_bounds.x, total_height),
        granted: None,
        token_type: TokenType::App,
        token: modal.authtoken,
    };
    log::debug!("applying recomputed bounds of {:?}", new_bounds);
    modal.gam.set_canvas_bounds_request(&mut new_bounds).expect("couldn't call set bounds");
}

impl<'a> Modal<'a> {
    pub fn new(name: &str, action: ActionType, top_text: Option<&str>, bot_text: Option<&str>, style: GlyphStyle, margin: i16) -> Modal<'a> {
        let xns = xous_names::XousNames::new().unwrap();
        let sid = xous::create_server().expect("can't create private modal message server");
        let gam = Gam::new(&xns).expect("can't connect to GAM");
        let authtoken = gam.register_ux(
            UxRegistration {
                app_name: String::<128>::from_str(name),
                ux_type: UxType::Modal,
                predictor: None,
                listener: sid.to_array(),
                redraw_id: ModalOpcode::Redraw.to_u32().unwrap(),
                gotinput_id: None,
                audioframe_id: None,
                rawkeys_id: Some(ModalOpcode::Rawkeys.to_u32().unwrap()),
            }
        ).expect("couldn't register my Ux element with GAM");
        assert!(authtoken.is_some(), "Couldn't register modal. Did you remember to add the app_name to the tokens.rs expected boot contexts list?");
        log::debug!("requesting content canvas for modal");
        let canvas = gam.request_content_canvas(authtoken.unwrap()).expect("couldn't get my content canvas from GAM");
        let line_height = if xous::LANG == "zh" {
            // zh has no "small" style
            gam.glyph_height_hint(GlyphStyle::Regular).expect("couldn't get glyph height hint") as i16
        } else {
            gam.glyph_height_hint(style).expect("couldn't get glyph height hint") as i16
        };
        let canvas_bounds = gam.get_canvas_bounds(canvas).expect("couldn't get starting canvas bounds");

        log::trace!("initializing Modal structure");
        // check to see if this is a password field or not
        // note: if a modal claims it's a password field but lacks sufficient trust level, the GAM will refuse
        // to render the element.
        let inverted = match action {
            ActionType::TextEntry(_) => action.is_password(),
            _ => false
        };

        // we now have a canvas that is some minimal height, but with the final width as allowed by the GAM.
        // compute the final height based upon the contents within.
        let mut modal = Modal {
            sid,
            gam,
            xns,
            top_text: None,
            bot_text: None,
            action,
            canvas,
            authtoken: authtoken.unwrap(),
            margin,
            line_height,
            canvas_width: canvas_bounds.x, // memoize this, it shouldn't change
            inverted,
            style,
            helper_data: None,
            name: String::<128>::from_str(name),
        };
        recompute_canvas(&mut modal, action, top_text, bot_text, style);
        modal
    }
    pub fn activate(&self) {
        self.gam.raise_modal(self.name.to_str()).expect("couldn't activate modal");
    }

    /// this function spawns a client-side thread to forward redraw and key event
    /// messages on to a local server. The goal is to keep the local server's SID
    /// a secret. The GAM only knows the single-use SID for redraw commands; this
    /// isolates a server's private command set from the GAM.
    pub fn spawn_helper(&mut self, private_sid: xous::SID, public_sid: xous::SID, redraw_op: u32, rawkeys_op: u32, drop_op: u32) {
        let helper_data = MsgForwarder {
            private_sid: private_sid.to_array(),
            public_sid: public_sid.to_array(),
            redraw_op,
            rawkeys_op,
            drop_op
        };
        let buf = Buffer::into_buf(helper_data).expect("couldn't allocate helper data for helper thread");
        let (addr, size, offset) = unsafe{buf.to_raw_parts()};
        self.helper_data = Some(buf);
        xous::create_thread_3(crate::forwarding_thread, addr, size, offset).expect("couldn't spawn a helper thread");
    }

    pub fn redraw(&self) {
        log::debug!("modal redraw");
        let canvas_size = self.gam.get_canvas_bounds(self.canvas).unwrap();
        // draw the outer border
        self.gam.draw_rounded_rectangle(self.canvas,
            RoundedRectangle::new(
                Rectangle::new_with_style(Point::new(0, 0), canvas_size,
                    DrawStyle::new(if self.inverted{PixelColor::Dark} else {PixelColor::Light}, PixelColor::Dark, 3)
                ), 5
            )).unwrap();

        let mut cur_height = self.margin;
        if let Some(mut tv) = self.top_text {
            self.gam.post_textview(&mut tv).expect("couldn't draw text");
            if let Some(bounds) = tv.bounds_computed {
                cur_height += bounds.br.y - bounds.tl.y;
            }
        }

        self.action.redraw(cur_height, &self);
        cur_height += self.action.height(self.line_height, self.margin);

        if let Some(mut tv) = self.bot_text {
            self.gam.post_textview(&mut tv).expect("couldn't draw text");
            if let Some(bounds) = tv.bounds_computed {
                cur_height += bounds.br.y - bounds.tl.y;
            }
        }
        log::trace!("total height: {}", cur_height);
        self.gam.redraw().unwrap();
    }

    pub fn key_event(&mut self, keys: [char; 4]) {
        for &k in keys.iter() {
            if k != '\u{0}' {
                log::debug!("got key '{}'", k);
                let (err, close) = self.action.key_action(k);
                if let Some(err_msg) = err {
                    self.modify(None, None, false, Some(err_msg.to_str()), false, None);
                } else {
                    if close {
                        log::debug!("closing modal");
                        // if it's a "close" button, invoke the GAM to put our box away
                        self.gam.relinquish_focus().unwrap();
                    }
                }
            }
        }
        self.redraw();
    }

    /// this function will modify UX elements if any of the arguments are Some()
    /// if None, the element is unchanged.
    /// If a text section is set to remove, but Some() is given for the update, the text is not removed, and instead replaced with the updated text.
    pub fn modify(&mut self, update_action: Option<ActionType>,
        update_top_text: Option<&str>, remove_top: bool,
        update_bot_text: Option<&str>, remove_bot: bool,
        update_style: Option<GlyphStyle>) {
        let action = if let Some(action) = update_action {
            self.action = action;
            action
        } else {
            self.action
        };

        if remove_top {
            self.top_text = None;
        }
        if remove_bot {
            self.bot_text = None;
        }

        let mut top_tv_temp = String::<3072>::new(); // size matches that used in TextView
        if let Some(top_text) = update_top_text {
            write!(top_tv_temp, "{}", top_text).unwrap();
        } else {
            if let Some(top_text) = self.top_text {
                write!(top_tv_temp, "{}", top_text).unwrap();
            }
        };
        let top_text = if self.top_text.is_none() && update_top_text.is_none() {
            None
        } else {
            Some(top_tv_temp.to_str())
        };

        let mut bot_tv_temp = String::<3072>::new(); // size matches that used in TextView
        if let Some(bot_text) = update_bot_text {
            write!(bot_tv_temp, "{}", bot_text).unwrap();
        } else {
            if let Some(bot_text) = self.bot_text {
                write!(bot_tv_temp, "{}", bot_text).unwrap();
            }
        };
        let bot_text = if self.bot_text.is_none() && update_bot_text.is_none() {
            None
        } else {
            Some(bot_tv_temp.to_str())
        };

        let style = if let Some(style) = update_style {
            style
        } else {
            self.style
        };
        recompute_canvas(self, action, top_text, bot_text, style);
    }
}