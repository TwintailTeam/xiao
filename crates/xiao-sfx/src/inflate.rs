use std::io::{BufRead, Error, ErrorKind, Read, Result};

const WINDOW: usize = 32 * 1024;
const CHUNK: usize = 256 * 1024;

const LENGTH_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LENGTH_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DISTANCE_BASE: [u16; 30] =
    [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DISTANCE_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
const CODE_LENGTH_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

fn damaged() -> Error { Error::new(ErrorKind::InvalidData, "the app inside this program is damaged") }

fn truncated() -> Error { Error::new(ErrorKind::UnexpectedEof, "the app inside this program is incomplete") }

pub struct Inflate<R> {
    bits: Bits<R>,
    window: Vec<u8>,
    pos: usize,
    block: Block,
    last: bool,
    adler: Adler,
    expected: Option<u32>,
}

enum Block {
    Zlib,
    Header,
    Stored(u16),
    Huffman(Table, Table),
}

impl<R: BufRead> Inflate<R> {
    pub fn new(source: R) -> Self {
        Self {
            bits: Bits { source, buf: 0, count: 0, padding: 0 },
            window: Vec::with_capacity(WINDOW + CHUNK + 258),
            pos: 0,
            block: Block::Zlib,
            last: false,
            adler: Adler { a: 1, b: 0 },
            expected: None,
        }
    }

    fn produce(&mut self) -> Result<()> {
        if self.window.len() > WINDOW {
            let cut = self.window.len() - WINDOW;
            self.window.drain(..cut);
            self.pos -= cut;
        }
        let goal = self.window.len() + CHUNK;
        while self.window.len() < goal && self.expected.is_none() {
            self.step(goal)?;
        }
        self.adler.update(&self.window[self.pos..]);
        match self.expected {
            Some(expected) if expected != self.adler.value() => Err(damaged()),
            _ => Ok(()),
        }
    }

    fn step(&mut self, goal: usize) -> Result<()> {
        let bits = &mut self.bits;
        let window = &mut self.window;
        let next = match &mut self.block {
            Block::Zlib => {
                let header = bits.take(16)?;
                let (method, flags) = (header & 0xff, header >> 8);
                if method & 0x0f != 8 || method >> 4 > 7 || flags & 0x20 != 0 || ((method << 8) | flags) % 31 != 0 {
                    return Err(damaged());
                }
                Block::Header
            }
            Block::Header if self.last => {
                bits.align();
                let mut expected = 0;
                for _ in 0..4 {
                    expected = (expected << 8) | bits.take(8)?;
                }
                self.expected = Some(expected);
                return Ok(());
            }
            Block::Header => {
                self.last = bits.take(1)? == 1;
                match bits.take(2)? {
                    0 => {
                        bits.align();
                        let len = bits.take(16)?;
                        if len != !bits.take(16)? & 0xffff {
                            return Err(damaged());
                        }
                        Block::Stored(len as u16)
                    }
                    1 => fixed()?,
                    2 => dynamic(bits)?,
                    _ => return Err(damaged()),
                }
            }
            Block::Stored(left) => {
                while *left > 0 && window.len() < goal {
                    window.push(bits.take(8)? as u8);
                    *left -= 1;
                }
                if *left > 0 {
                    return Ok(());
                }
                Block::Header
            }
            Block::Huffman(literals, distances) => {
                loop {
                    if window.len() >= goal {
                        return Ok(());
                    }
                    let symbol = usize::from(bits.decode(literals)?);
                    if symbol < 256 {
                        window.push(symbol as u8);
                        continue;
                    }
                    if symbol == 256 {
                        break;
                    }
                    let index = symbol - 257;
                    if index >= LENGTH_BASE.len() {
                        return Err(damaged());
                    }
                    let length = usize::from(LENGTH_BASE[index]) + bits.take(u32::from(LENGTH_EXTRA[index]))? as usize;
                    let index = usize::from(bits.decode(distances)?);
                    if index >= DISTANCE_BASE.len() {
                        return Err(damaged());
                    }
                    let distance = usize::from(DISTANCE_BASE[index]) + bits.take(u32::from(DISTANCE_EXTRA[index]))? as usize;
                    if distance > window.len() {
                        return Err(damaged());
                    }
                    let start = window.len() - distance;
                    if distance >= length {
                        window.extend_from_within(start..start + length);
                    } else {
                        for at in start..start + length {
                            window.push(window[at]);
                        }
                    }
                }
                Block::Header
            }
        };
        self.block = next;
        Ok(())
    }
}

impl<R: BufRead> Read for Inflate<R> {
    fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        if self.pos == self.window.len() {
            if self.expected.is_some() {
                return Ok(0);
            }
            self.produce()?;
        }
        let count = out.len().min(self.window.len() - self.pos);
        out[..count].copy_from_slice(&self.window[self.pos..self.pos + count]);
        self.pos += count;
        Ok(count)
    }
}

fn fixed() -> Result<Block> {
    let mut lengths = [0; 288];
    lengths[..144].fill(8);
    lengths[144..256].fill(9);
    lengths[256..280].fill(7);
    lengths[280..].fill(8);
    Ok(Block::Huffman(Table::new(&lengths)?, Table::new(&[5; 30])?))
}

fn dynamic<R: BufRead>(bits: &mut Bits<R>) -> Result<Block> {
    let literal_count = bits.take(5)? as usize + 257;
    let distance_count = bits.take(5)? as usize + 1;
    let code_count = bits.take(4)? as usize + 4;
    if literal_count > 286 || distance_count > 30 {
        return Err(damaged());
    }

    let mut code_lengths = [0; 19];
    for &symbol in &CODE_LENGTH_ORDER[..code_count] {
        code_lengths[symbol] = bits.take(3)? as u8;
    }
    let codes = Table::new(&code_lengths)?;

    let mut lengths = [0u8; 286 + 30];
    let total = literal_count + distance_count;
    let mut at = 0;
    while at < total {
        let (value, repeat) = match bits.decode(&codes)? {
            symbol @ 0..=15 => (symbol as u8, 1),
            16 if at > 0 => (lengths[at - 1], 3 + bits.take(2)? as usize),
            17 => (0, 3 + bits.take(3)? as usize),
            18 => (0, 11 + bits.take(7)? as usize),
            _ => return Err(damaged()),
        };
        if at + repeat > total {
            return Err(damaged());
        }
        lengths[at..at + repeat].fill(value);
        at += repeat;
    }
    if lengths[256] == 0 {
        return Err(damaged());
    }
    Ok(Block::Huffman(Table::new(&lengths[..literal_count])?, Table::new(&lengths[literal_count..total])?))
}

struct Table {
    entries: Vec<u16>,
    mask: u64,
}

impl Table {
    fn new(lengths: &[u8]) -> Result<Self> {
        let mut counts = [0u16; 16];
        for &length in lengths {
            counts[usize::from(length)] += 1;
        }
        counts[0] = 0;

        let mut left = 1i32;
        let mut next = [0u16; 16];
        let mut code = 0u16;
        for length in 1..16 {
            left = (left << 1) - i32::from(counts[length]);
            if left < 0 {
                return Err(damaged());
            }
            code = (code + counts[length - 1]) << 1;
            next[length] = code;
        }

        let width = lengths.iter().copied().max().unwrap_or(0);
        let mut entries = vec![0u16; 1 << width];
        for (symbol, &length) in lengths.iter().enumerate() {
            if length == 0 {
                continue;
            }
            let code = next[usize::from(length)];
            next[usize::from(length)] += 1;
            let mut at = usize::from(code.reverse_bits() >> (16 - length));
            while at < entries.len() {
                entries[at] = (symbol as u16) << 4 | u16::from(length);
                at += 1 << length;
            }
        }
        Ok(Self { entries, mask: (1 << width) - 1 })
    }
}

struct Bits<R> {
    source: R,
    buf: u64,
    count: u32,
    padding: u32,
}

impl<R: BufRead> Bits<R> {
    fn refill(&mut self) -> Result<()> {
        while self.count <= 56 {
            let chunk = self.source.fill_buf()?;
            if chunk.is_empty() {
                self.count += 8;
                self.padding += 8;
                continue;
            }
            let count = (((64 - self.count) / 8) as usize).min(chunk.len());
            for (index, &byte) in chunk[..count].iter().enumerate() {
                self.buf |= u64::from(byte) << (self.count + 8 * index as u32);
            }
            self.count += 8 * count as u32;
            self.source.consume(count);
        }
        Ok(())
    }

    fn consume(&mut self, count: u32) -> Result<()> {
        if count + self.padding > self.count {
            return Err(truncated());
        }
        self.buf >>= count;
        self.count -= count;
        Ok(())
    }

    fn take(&mut self, count: u32) -> Result<u32> {
        if count == 0 {
            return Ok(0);
        }
        if self.count < count {
            self.refill()?;
        }
        let value = (self.buf & ((1 << count) - 1)) as u32;
        self.consume(count)?;
        Ok(value)
    }

    fn decode(&mut self, table: &Table) -> Result<u16> {
        if self.count < 15 {
            self.refill()?;
        }
        let entry = table.entries[(self.buf & table.mask) as usize];
        let length = u32::from(entry & 0x0f);
        if length == 0 {
            return Err(damaged());
        }
        self.consume(length)?;
        Ok(entry >> 4)
    }

    fn align(&mut self) {
        let extra = self.count % 8;
        self.buf >>= extra;
        self.count -= extra;
    }
}

struct Adler {
    a: u32,
    b: u32,
}

impl Adler {
    fn update(&mut self, data: &[u8]) {
        for chunk in data.chunks(5552) {
            for &byte in chunk {
                self.a += u32::from(byte);
                self.b += self.a;
            }
            self.a %= 65521;
            self.b %= 65521;
        }
    }

    fn value(&self) -> u32 { (self.b << 16) | self.a }
}
