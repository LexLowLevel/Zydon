// Bit manipulation utilities.

/// Count leading zeros.
pub fn clz(x: u32) -> u32 {
    x.leading_zeros()
}

/// Count trailing zeros.
pub fn ctz(x: u32) -> u32 {
    x.trailing_zeros()
}

/// Extract a single bit at position `bit` from `x`.
pub const fn bit(x: u32, bit: u32) -> u32 {
    x & (1 << bit)
}

/// Test if bit `bit` is set in `x`, returning 0 or 1.
pub const fn bit_shift(x: u32, bit: u32) -> u32 {
    (x >> bit) & 1
}

/// Extract bits [high:low] (inclusive) from `x` without shifting.
pub const fn bits(x: u32, high: u32, low: u32) -> u32 {
    let mask = ((1u32 << (high + 1)) - 1) & !((1u32 << low) - 1);
    x & mask
}

/// Extract bits [high:low] (inclusive) from `x`, shifted down to LSB.
pub const fn bits_shift(x: u32, high: u32, low: u32) -> u32 {
    let mask = (1u32 << (high - low + 1)) - 1;
    (x >> low) & mask
}

/// Test if bit `bit` is set in `x`, returning 1 if set, 0 otherwise.
pub const fn bit_set(x: u32, bit: u32) -> u32 {
    if (x & (1 << bit)) != 0 { 1 } else { 0 }
}

// --- 64-bit variants ---

pub const fn bit64(x: u64, bit: u32) -> u64 {
    x & (1u64 << bit)
}

pub const fn bit_shift64(x: u64, bit: u32) -> u64 {
    (x >> bit) & 1
}

pub const fn bits64(x: u64, high: u32, low: u32) -> u64 {
    let mask = ((1u64 << (high + 1)) - 1) & !((1u64 << low) - 1);
    x & mask
}

pub const fn bits_shift64(x: u64, high: u32, low: u32) -> u64 {
    let mask = (1u64 << (high - low + 1)) - 1;
    (x >> low) & mask
}

pub const fn bit_set64(x: u64, bit: u32) -> u64 {
    if (x & (1u64 << bit)) != 0 { 1 } else { 0 }
}

// --- Bitmap constants ---

pub const BITMAP_BITS_PER_WORD: usize = core::mem::size_of::<usize>() * 8;
pub const BITMAP_BITS_PER_INT: usize = core::mem::size_of::<u32>() * 8;

pub const fn bitmap_num_words(x: usize) -> usize {
    (x + BITMAP_BITS_PER_WORD - 1) / BITMAP_BITS_PER_WORD
}

pub const fn bitmap_word(x: usize) -> usize {
    x / BITMAP_BITS_PER_WORD
}

pub const fn bitmap_bit_in_word(x: usize) -> usize {
    x & (BITMAP_BITS_PER_WORD - 1)
}

pub const fn bitmap_int(x: usize) -> usize {
    x / BITMAP_BITS_PER_INT
}

pub const fn bitmap_bit_in_int(x: usize) -> usize {
    x & (BITMAP_BITS_PER_INT - 1)
}

pub const fn bitmap_first_word_mask(start: usize) -> usize {
    !0usize << (start % BITMAP_BITS_PER_WORD)
}

pub const fn bitmap_last_word_mask(nbits: usize) -> usize {
    if nbits % BITMAP_BITS_PER_WORD != 0 {
        (1usize << (nbits % BITMAP_BITS_PER_WORD)) - 1
    } else {
        !0usize
    }
}

// --- Bit masks ---

pub const fn bit_mask(x: usize) -> usize {
    if x >= core::mem::size_of::<usize>() * 8 {
        !0usize
    } else {
        (1usize << x) - 1
    }
}

pub const fn bit_mask32(x: u32) -> u32 {
    if x >= 32 {
        !0u32
    } else {
        (1u32 << x) - 1
    }
}

// --- Bitmap operations ---

/// Set `nr` bits starting from `start` in `bitmap`.
pub fn bitmap_set(bitmap: &mut [usize], start: usize, nr: usize) {
    let mut p = bitmap_word(start);
    let size = start + nr;
    let mut bits_to_set = BITMAP_BITS_PER_WORD - (start % BITMAP_BITS_PER_WORD);
    let mut mask_to_set: usize = bitmap_first_word_mask(start);
    let mut remaining = nr;

    while remaining >= bits_to_set {
        bitmap[p] |= mask_to_set;
        remaining -= bits_to_set;
        bits_to_set = BITMAP_BITS_PER_WORD;
        mask_to_set = !0usize;
        p += 1;
    }
    if remaining > 0 {
        bitmap[p] |= mask_to_set & bitmap_last_word_mask(size);
    }
}

/// Clear `nr` bits starting from `start` in `bitmap`.
pub fn bitmap_clear(bitmap: &mut [usize], start: usize, nr: usize) {
    let mut p = bitmap_word(start);
    let size = start + nr;
    let mut bits_to_clear = BITMAP_BITS_PER_WORD - (start % BITMAP_BITS_PER_WORD);
    let mut mask_to_clear: usize = bitmap_first_word_mask(start);
    let mut remaining = nr;

    while remaining >= bits_to_clear {
        bitmap[p] &= !mask_to_clear;
        remaining -= bits_to_clear;
        bits_to_clear = BITMAP_BITS_PER_WORD;
        mask_to_clear = !0usize;
        p += 1;
    }
    if remaining > 0 {
        bitmap[p] &= !(mask_to_clear & bitmap_last_word_mask(size));
    }
}

/// Test if bit `bit` is set in `bitmap`.
pub fn bitmap_test(bitmap: &[usize], bit: usize) -> bool {
    (bitmap[bitmap_word(bit)] & (1usize << bitmap_bit_in_word(bit))) != 0
}

/// Find first zero bit starting from LSB.
pub fn ffz(x: usize) -> usize {
    (!x).trailing_zeros() as usize
}

/// Find first zero bit in `bitmap` with `numbits` total bits.
/// Returns the bit index, or None if all bits are set.
pub fn bitmap_ffz(bitmap: &[usize], numbits: usize) -> Option<usize> {
    for i in 0..bitmap_num_words(numbits) {
        if bitmap[i] == !0usize {
            continue;
        }
        let bit = i * BITMAP_BITS_PER_WORD + ffz(bitmap[i]);
        if bit < numbits {
            return Some(bit);
        }
        return None;
    }
    None
}
