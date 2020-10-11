use core::fmt::{self, Debug, Write};
use spin::Mutex;
use crate::memory::KERNEL_MAPPING_BEGIN;
use core::ptr::NonNull;
use core::{ptr, cmp};

/// Represents colours, based off of VGA's colour set
#[allow(dead_code)] // dead variants for completeness
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[repr(u8)]
pub enum Colour {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct ColourPair {
    pub foreground: Colour,
    pub background: Colour,
}

impl ColourPair {
    #[allow(dead_code)] // Completeness
    pub const fn new(foreground: Colour, background: Colour) -> Self {
        ColourPair { foreground, background }
    }
}

impl Default for ColourPair {
    fn default() -> Self {
        ColourPair {
            foreground: Colour::White,
            background: Colour::Black
        }
    }
}

#[macro_export]
macro_rules! colour {
    ($foreground:ident, $background:ident) => {
        crate::vga::ColourPair {
            foreground: crate::vga::Colour::$foreground,
            background: crate::vga::Colour::$background,
        }
    };

    ($foreground:ident on $background:ident) => {
        colour!($foreground, $background)
    };
}

/// Writes formatted string to stdout, for print macro use
#[allow(dead_code)]
pub fn stdout_print(args: fmt::Arguments) {
    VGA_WRITER.lock().write_fmt(args).unwrap();
}

pub static VGA_WRITER: Mutex<VgaWriter> = Mutex::new(unsafe { VgaWriter::new() });

/// Represents a vga's resolution.
///
/// # Note
/// although the fields are the same as [Point], they *are* semantically different
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Resolution {
    pub x: usize,
    pub y: usize,
}

pub const VIRTUAL_VGA_PTR: u64 = KERNEL_MAPPING_BEGIN + 0xb8000;

/// The resolution of VGA
pub const RESOLUTION: Resolution = Resolution { x: 80, y: 25 };

/// Interface to VGA, allowing write
pub struct VgaWriter {
    buffer: NonNull<VgaBuffer>,
    cursor: (usize, usize),
    pub colour: ColourPair,
}

impl fmt::Debug for VgaWriter {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "VgaWriter")
    }
}

impl fmt::Write for VgaWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_str(s);
        Ok(())
    }
}

// Safety: to create, the vga writer must be unique.
unsafe impl Send for VgaWriter {}
unsafe impl Sync for VgaWriter {}

impl VgaWriter {
    /// # Safety
    ///
    /// Must be the only VGA writer in existence.
    pub const unsafe fn new() -> Self {
        VgaWriter {
            buffer: NonNull::new_unchecked(VIRTUAL_VGA_PTR as *mut _),
            cursor: (0, RESOLUTION.y - 1),
            colour: colour!(White on Black),
        }
    }

    fn buffer(&mut self) -> &mut VgaBuffer {
        unsafe { self.buffer.as_mut() }
    }
}

impl VgaWriter {
    fn set_char(&mut self, character: char, colour: ColourPair, point: (usize, usize)) {
        self.buffer().set_char(
            point.0,
            RESOLUTION.y - 1 - point.1,
            VgaChar::new(
                colour.into(),
                character as u8
            )
        );
    }

    pub fn write_str(&mut self, txt: &str) {
        self.write_str_coloured(txt, self.colour)
    }

    pub fn write_str_coloured(&mut self, txt: &str, colour: ColourPair) {
        for c in txt.chars() {
            self.write_coloured(c, colour)
        }
    }

    pub fn write_coloured(&mut self, character: char, colour: ColourPair) {
        match character {
            '\n' => self.new_line(),
            _ => {
                self.set_char(character, colour, self.cursor);
                self.cursor.0 += 1;

                // If the x point went out of bounds, wrap
                if self.cursor.0 >= RESOLUTION.x {
                    self.new_line();
                }
            }
        }
    }

    /// Writes a newline to this terminal, resetting cursor position
    fn new_line(&mut self) {
        self.cursor.0 = 0;
        if self.cursor.1 > 0 {
            self.cursor.1 -= 1;
        } else {
            self.scroll_down(1);
        }
    }

    fn scroll_down(&mut self, amount: usize) {
        let background = self.colour.background;
        self.buffer().scroll_down(amount, background);
    }

    pub fn clear(&mut self) {
        let background = self.colour.background;
        for line in 0..RESOLUTION.y {
            self.buffer().clear_row(line, background);
        }
    }
}

/// Represents the complete VGA character buffer, containing a 2D array of VgaChar
#[repr(C)]
struct VgaBuffer([[VgaChar; RESOLUTION.x]; RESOLUTION.y]);

impl VgaBuffer {
    pub fn set_char(&mut self, x: usize, y: usize, value: VgaChar) {
        unsafe { ptr::write_volatile(&mut self.0[y][x] as *mut _, value) }
    }

    pub fn scroll_down(&mut self, amount: usize, background_colour: Colour) {
        // Shift lines left (up) by amount only if amount < Y resolution
        // If amount is any more then the data will be cleared anyway
        if cmp::min(amount, RESOLUTION.y) < RESOLUTION.y {
            self.0.rotate_left(amount);
        }

        // Clear rows up to the amount
        for row in 0..amount {
            self.clear_row((RESOLUTION.y - 1) - row, background_colour);
        }
    }

    pub fn clear_row(&mut self, y: usize, colour: Colour) {
        let blank = VgaChar::new(
            VgaColour::new(Colour::Black, colour),
            b' '
        );

        for x in 0..RESOLUTION.x {
            self.set_char(x, y, blank);
        }
    }
}

/// Represents a full character in the VGA buffer, with a character code, foreground and background
#[derive(Copy, Clone)]
#[repr(C)]
pub struct VgaChar {
    pub character: u8,
    pub colour: VgaColour,
}

impl VgaChar {
    fn new(colour: VgaColour, character: u8) -> Self {
        VgaChar { colour, character }
    }
}

/// Represents a VGA colour, with both a foreground and background
#[derive(Clone, Copy)]
#[repr(C)]
pub struct VgaColour(u8);

impl VgaColour {
    /// Creates a new VgaColour for the given foreground and background
    pub const fn new(foreground: Colour, background: Colour) -> Self {
        VgaColour((background as u8) << 4 | (foreground as u8))
    }
}

impl From<ColourPair> for VgaColour {
    fn from(colour: ColourPair) -> VgaColour {
        VgaColour::new(colour.foreground, colour.background)
    }
}
