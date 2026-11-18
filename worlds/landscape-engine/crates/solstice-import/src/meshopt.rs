//! Decodes `EXT_meshopt_compression`, the buffer compression that gltfpack writes.
//!
//! The format is the extension's Appendix A: attributes are byte deltas between consecutive
//! elements, in blocks; triangles are one code byte each, leaning on two small histories of
//! recent edges and vertices. Filters are not implemented, and a file that uses one is refused
//! rather than read wrongly.

use gltf::buffer;

/// Why a compressed buffer view could not be decoded.
#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The stream ends early, or has bytes left over at its end.
    Malformed,
    /// A filter, mode, stride or version this decoder doesn't take.
    Unsupported(String),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed => write!(f, "the compressed stream is malformed"),
            Self::Unsupported(what) => write!(f, "unsupported compression: {what}"),
        }
    }
}

impl std::error::Error for DecodeError {}

const ATTRIBUTES_HEADER: u8 = 0xa0;
const TRIANGLES_HEADER: u8 = 0xe1;
const INDICES_HEADER: u8 = 0xd1;
/// Attribute streams end with a block at least this long, holding the baseline element last.
const ATTRIBUTES_TAIL: usize = 32;
/// Triangle streams end with this long a lookup table.
const TRIANGLES_TAIL: usize = 16;
const GROUP: usize = 16;

/// Reads forward through `data[..end]`.
struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
    end: usize,
}

impl Cursor<'_> {
    #[inline]
    fn byte(&mut self) -> Result<u8, DecodeError> {
        if self.at >= self.end {
            return Err(DecodeError::Malformed);
        }
        let byte = self.data[self.at];
        self.at += 1;
        Ok(byte)
    }

    fn bytes(&mut self, count: usize) -> Result<&[u8], DecodeError> {
        let start = self.at;
        let end = start.checked_add(count).ok_or(DecodeError::Malformed)?;
        if end > self.end {
            return Err(DecodeError::Malformed);
        }
        self.at = end;
        Ok(&self.data[start..end])
    }

    /// An unsigned LEB128 value, as the triangle and index codecs store numbers.
    fn varint(&mut self) -> Result<u32, DecodeError> {
        let mut value = 0u32;
        for shift in (0..35).step_by(7) {
            let byte = self.byte()?;
            value |= u32::from(byte & 0x7f).wrapping_shl(shift);
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(DecodeError::Malformed)
    }
}

#[inline]
fn unzigzag_byte(value: u8) -> u8 {
    (value >> 1) ^ 0u8.wrapping_sub(value & 1)
}

#[inline]
fn unzigzag(value: u32) -> u32 {
    (value >> 1) ^ 0u32.wrapping_sub(value & 1)
}

/// Decodes the 16 deltas of one group, as stored in sentinel mode `mode`.
fn decode_group(
    mode: u8,
    cursor: &mut Cursor<'_>,
    deltas: &mut [u8; GROUP],
) -> Result<(), DecodeError> {
    match mode {
        0 => deltas.fill(0),
        1 => {
            let selectors: [u8; 4] = cursor.bytes(4)?.try_into().expect("four bytes");
            for (i, delta) in deltas.iter_mut().enumerate() {
                let bits = selectors[i / 4] >> (6 - (i % 4) * 2) & 3;
                *delta = if bits == 3 { cursor.byte()? } else { bits };
            }
        }
        2 => {
            let selectors: [u8; 8] = cursor.bytes(8)?.try_into().expect("eight bytes");
            for (i, delta) in deltas.iter_mut().enumerate() {
                let bits = selectors[i / 2] >> (4 - (i % 2) * 4) & 15;
                *delta = if bits == 15 { cursor.byte()? } else { bits };
            }
        }
        _ => deltas.copy_from_slice(cursor.bytes(GROUP)?),
    }
    Ok(())
}

/// Decodes a stream of mode `ATTRIBUTES` into `out`, which holds a whole number of elements of
/// `stride` bytes.
pub fn decode_attributes(out: &mut [u8], stride: usize, data: &[u8]) -> Result<(), DecodeError> {
    if stride == 0 || !stride.is_multiple_of(4) || stride > 256 || !out.len().is_multiple_of(stride)
    {
        return Err(DecodeError::Unsupported(format!(
            "attribute stride {stride}"
        )));
    }
    match data.first() {
        Some(&ATTRIBUTES_HEADER) => {}
        Some(&other) => {
            return Err(DecodeError::Unsupported(format!(
                "attribute stream version {other:#x}"
            )))
        }
        None => return Err(DecodeError::Malformed),
    }
    let tail = stride.max(ATTRIBUTES_TAIL);
    if data.len() < 1 + tail {
        return Err(DecodeError::Malformed);
    }
    let end = data.len() - tail;
    let mut cursor = Cursor { data, at: 1, end };
    let mut last = [0u8; 256];
    last[..stride].copy_from_slice(&data[data.len() - stride..]);

    let count = out.len() / stride;
    let block_limit = ((8192 / stride) & !15).min(256);
    let mut deltas = [[0u8; GROUP]; 16];
    let mut done = 0;
    while done < count {
        let elements = (count - done).min(block_limit);
        let groups = elements.div_ceil(GROUP);
        for byte in 0..stride {
            let start = cursor.at;
            cursor.bytes(groups.div_ceil(4))?;
            for (group, deltas) in deltas[..groups].iter_mut().enumerate() {
                let header = data[start + group / 4];
                decode_group(header >> (group % 4 * 2) & 3, &mut cursor, deltas)?;
            }
            let mut value = last[byte];
            for element in 0..elements {
                let delta = deltas[element / GROUP][element % GROUP];
                value = value.wrapping_add(unzigzag_byte(delta));
                out[(done + element) * stride + byte] = value;
            }
        }
        last[..stride].copy_from_slice(&out[(done + elements - 1) * stride..][..stride]);
        done += elements;
    }
    if cursor.at != end {
        return Err(DecodeError::Malformed);
    }
    Ok(())
}

/// The recent history a triangle stream refers to.
struct History {
    edges: [[u32; 2]; GROUP],
    edge_at: usize,
    vertices: [u32; GROUP],
    vertex_at: usize,
}

impl History {
    #[inline]
    fn edge(&self, age: usize) -> [u32; 2] {
        self.edges[self.edge_at.wrapping_sub(1 + age) & 15]
    }

    #[inline]
    fn vertex(&self, age: usize) -> u32 {
        self.vertices[self.vertex_at.wrapping_sub(1 + age) & 15]
    }

    #[inline]
    fn push_edge(&mut self, a: u32, b: u32) {
        self.edges[self.edge_at] = [a, b];
        self.edge_at = (self.edge_at + 1) & 15;
    }

    #[inline]
    fn push_vertex(&mut self, v: u32) {
        self.vertices[self.vertex_at] = v;
        self.vertex_at = (self.vertex_at + 1) & 15;
    }
}

fn write_index(out: &mut [u8], stride: usize, index: usize, value: u32) {
    if stride == 2 {
        out[index * 2..index * 2 + 2].copy_from_slice(&(value as u16).to_le_bytes());
    } else {
        out[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
}

/// Decodes a stream of mode `TRIANGLES` into `out`, as indices of `stride` bytes.
pub fn decode_triangles(out: &mut [u8], stride: usize, data: &[u8]) -> Result<(), DecodeError> {
    if (stride != 2 && stride != 4)
        || !out.len().is_multiple_of(stride)
        || !(out.len() / stride).is_multiple_of(3)
    {
        return Err(DecodeError::Unsupported(format!(
            "triangle stride {stride}"
        )));
    }
    match data.first() {
        Some(&TRIANGLES_HEADER) => {}
        Some(&other) => {
            return Err(DecodeError::Unsupported(format!(
                "triangle stream version {other:#x}"
            )))
        }
        None => return Err(DecodeError::Malformed),
    }
    let triangles = out.len() / stride / 3;
    if data.len() < 1 + triangles + TRIANGLES_TAIL {
        return Err(DecodeError::Malformed);
    }
    let end = data.len() - TRIANGLES_TAIL;
    let codes = &data[1..1 + triangles];
    let table = &data[end..];
    let mut extra = Cursor {
        data,
        at: 1 + triangles,
        end,
    };
    let mut history = History {
        edges: [[0; 2]; GROUP],
        edge_at: 0,
        vertices: [0; GROUP],
        vertex_at: 0,
    };
    let mut next = 0u32;
    let mut last = 0u32;

    for (triangle, &code) in codes.iter().enumerate() {
        let high = usize::from(code >> 4);
        let low = code & 15;
        let (a, b, c);
        if high < 15 {
            // A recent edge, and one more vertex.
            [a, b] = history.edge(high);
            let (third, is_new) = match low {
                0 => {
                    next = next.wrapping_add(1);
                    (next.wrapping_sub(1), true)
                }
                13 => {
                    last = last.wrapping_sub(1);
                    (last, true)
                }
                14 => {
                    last = last.wrapping_add(1);
                    (last, true)
                }
                15 => {
                    last = last.wrapping_add(unzigzag(extra.varint()?));
                    (last, true)
                }
                age => (history.vertex(usize::from(age)), false),
            };
            c = third;
            history.push_edge(c, b);
            history.push_edge(a, c);
            if is_new {
                history.push_vertex(c);
            }
        } else if low < 14 {
            // Three vertices, by a line of the lookup table.
            let entry = table[usize::from(low)];
            let (z, w) = (usize::from(entry >> 4), usize::from(entry & 15));
            a = next;
            next = next.wrapping_add(1);
            b = if z == 0 {
                next = next.wrapping_add(1);
                next.wrapping_sub(1)
            } else {
                history.vertex(z - 1)
            };
            c = if w == 0 {
                next = next.wrapping_add(1);
                next.wrapping_sub(1)
            } else {
                history.vertex(w - 1)
            };
            history.push_edge(b, a);
            history.push_edge(c, b);
            history.push_edge(a, c);
            history.push_vertex(a);
            if z == 0 {
                history.push_vertex(b);
            }
            if w == 0 {
                history.push_vertex(c);
            }
        } else {
            // Three vertices, spelled out.
            let entry = extra.byte()?;
            if entry == 0 {
                next = 0;
            }
            let (z, w) = (usize::from(entry >> 4), usize::from(entry & 15));
            let explicit = |extra: &mut Cursor<'_>, last: &mut u32| -> Result<u32, DecodeError> {
                *last = last.wrapping_add(unzigzag(extra.varint()?));
                Ok(*last)
            };
            a = if low == 14 {
                next = next.wrapping_add(1);
                next.wrapping_sub(1)
            } else {
                explicit(&mut extra, &mut last)?
            };
            b = match z {
                0 => {
                    next = next.wrapping_add(1);
                    next.wrapping_sub(1)
                }
                15 => explicit(&mut extra, &mut last)?,
                age => history.vertex(age - 1),
            };
            c = match w {
                0 => {
                    next = next.wrapping_add(1);
                    next.wrapping_sub(1)
                }
                15 => explicit(&mut extra, &mut last)?,
                age => history.vertex(age - 1),
            };
            history.push_edge(b, a);
            history.push_edge(c, b);
            history.push_edge(a, c);
            history.push_vertex(a);
            if z == 0 || z == 15 {
                history.push_vertex(b);
            }
            if w == 0 || w == 15 {
                history.push_vertex(c);
            }
        }
        write_index(out, stride, triangle * 3, a);
        write_index(out, stride, triangle * 3 + 1, b);
        write_index(out, stride, triangle * 3 + 2, c);
    }
    if extra.at != end {
        return Err(DecodeError::Malformed);
    }
    Ok(())
}

/// Decodes a stream of mode `INDICES` into `out`, as indices of `stride` bytes.
pub fn decode_indices(out: &mut [u8], stride: usize, data: &[u8]) -> Result<(), DecodeError> {
    const TAIL: usize = 4;
    if (stride != 2 && stride != 4) || !out.len().is_multiple_of(stride) {
        return Err(DecodeError::Unsupported(format!("index stride {stride}")));
    }
    match data.first() {
        Some(&INDICES_HEADER) => {}
        Some(&other) => {
            return Err(DecodeError::Unsupported(format!(
                "index stream version {other:#x}"
            )))
        }
        None => return Err(DecodeError::Malformed),
    }
    if data.len() < 1 + TAIL {
        return Err(DecodeError::Malformed);
    }
    let end = data.len() - TAIL;
    let mut cursor = Cursor { data, at: 1, end };
    let mut baselines = [0u32; 2];
    for index in 0..out.len() / stride {
        let value = cursor.varint()?;
        let baseline = (value & 1) as usize;
        let delta = unzigzag(value >> 1);
        baselines[baseline] = baselines[baseline].wrapping_add(delta);
        write_index(out, stride, index, baselines[baseline]);
    }
    if cursor.at != end {
        return Err(DecodeError::Malformed);
    }
    Ok(())
}

/// Decompresses one buffer view into its place in `buffers`, if it is compressed. `buffers` must
/// already hold the compressed buffer and room for the view's uncompressed data.
pub fn decompress_view(
    view: &buffer::View<'_>,
    buffers: &mut [buffer::Data],
) -> Result<(), DecodeError> {
    let number = |value: &gltf::json::Value, name: &str, default: Option<u64>| {
        value
            .get(name)
            .and_then(gltf::json::Value::as_u64)
            .or(default)
            .map(|n| n as usize)
            .ok_or(DecodeError::Malformed)
    };
    let Some(extension) = view.extension_value("EXT_meshopt_compression") else {
        return Ok(());
    };
    let source = number(extension, "buffer", None)?;
    let offset = number(extension, "byteOffset", Some(0))?;
    let length = number(extension, "byteLength", None)?;
    let stride = number(extension, "byteStride", None)?;
    let count = number(extension, "count", None)?;
    let mode = extension.get("mode").and_then(gltf::json::Value::as_str);
    let filter = extension
        .get("filter")
        .and_then(gltf::json::Value::as_str)
        .unwrap_or("NONE");
    if filter != "NONE" {
        return Err(DecodeError::Unsupported(format!("filter {filter}")));
    }
    let size = stride.checked_mul(count).ok_or(DecodeError::Malformed)?;
    let target = view.buffer().index();
    if target == source {
        return Err(DecodeError::Malformed);
    }
    // Moved out while decoding, so the output goes straight to where it belongs.
    let mut output = std::mem::take(&mut buffers.get_mut(target).ok_or(DecodeError::Malformed)?.0);
    let result = (|| {
        let out = view
            .offset()
            .checked_add(size)
            .and_then(|end| output.get_mut(view.offset()..end))
            .ok_or(DecodeError::Malformed)?;
        let compressed = buffers
            .get(source)
            .and_then(|b| b.get(offset..offset.checked_add(length)?))
            .ok_or(DecodeError::Malformed)?;
        match mode {
            Some("ATTRIBUTES") => decode_attributes(out, stride, compressed),
            Some("TRIANGLES") => decode_triangles(out, stride, compressed),
            Some("INDICES") => decode_indices(out, stride, compressed),
            other => Err(DecodeError::Unsupported(format!("mode {other:?}"))),
        }
    })();
    buffers[target].0 = output;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zigzag_byte(delta: u8) -> u8 {
        if delta & 0x80 != 0 {
            !(delta << 1)
        } else {
            delta << 1
        }
    }

    /// Encodes one group in the sentinel mode given.
    fn encode_group(mode: u8, deltas: &[u8; GROUP], out: &mut Vec<u8>) {
        match mode {
            0 => assert!(deltas.iter().all(|&d| d == 0)),
            1 | 2 => {
                let (bits, sentinel) = if mode == 1 { (2, 3) } else { (4, 15) };
                let per_byte = 8 / bits;
                let mut selectors = vec![0u8; GROUP / per_byte];
                let mut extra = Vec::new();
                for (i, &delta) in deltas.iter().enumerate() {
                    let code = if delta < sentinel {
                        delta
                    } else {
                        extra.push(delta);
                        sentinel
                    };
                    selectors[i / per_byte] |= code << (8 - bits - (i % per_byte) * bits);
                }
                out.extend(selectors);
                out.extend(extra);
            }
            _ => out.extend_from_slice(deltas),
        }
    }

    /// Encodes `source`, trying every usable group mode in turn so that all of them are read back.
    fn encode_attributes(source: &[u8], stride: usize) -> Vec<u8> {
        let mut out = vec![ATTRIBUTES_HEADER];
        let baseline = source[..stride].to_vec();
        let mut last = baseline.clone();
        let count = source.len() / stride;
        let limit = ((8192 / stride) & !15).min(256);
        let mut turn = 0u8;
        let mut done = 0;
        while done < count {
            let elements = (count - done).min(limit);
            let groups = elements.div_ceil(GROUP);
            for byte in 0..stride {
                let mut deltas = vec![[0u8; GROUP]; groups];
                let mut previous = last[byte];
                for element in 0..elements {
                    let value = source[(done + element) * stride + byte];
                    deltas[element / GROUP][element % GROUP] =
                        zigzag_byte(value.wrapping_sub(previous));
                    previous = value;
                }
                let mut header = vec![0u8; groups.div_ceil(4)];
                let mut body = Vec::new();
                for (group, deltas) in deltas.iter().enumerate() {
                    let mode = if deltas.iter().all(|&d| d == 0) && turn.is_multiple_of(2) {
                        0
                    } else {
                        1 + turn % 3
                    };
                    turn = turn.wrapping_add(1);
                    header[group / 4] |= mode << (group % 4 * 2);
                    encode_group(mode, deltas, &mut body);
                }
                out.extend(header);
                out.extend(body);
            }
            last.copy_from_slice(&source[(done + elements - 1) * stride..][..stride]);
            done += elements;
        }
        out.extend(vec![0; stride.max(ATTRIBUTES_TAIL) - stride]);
        out.extend(baseline);
        out
    }

    #[test]
    fn attributes_survive_a_round_trip_in_every_group_mode() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut random = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for (stride, count) in [
            (4, 1),
            (4, 15),
            (4, 16),
            (4, 17),
            (12, 1000),
            (64, 200),
            (256, 40),
        ] {
            let mut source = vec![0u8; stride * count];
            for (i, byte) in source.iter_mut().enumerate() {
                // Runs, small steps and noise, so each group mode meets values it suits.
                *byte = match (i / stride / 16) % 3 {
                    0 => 7,
                    1 => (i / stride) as u8 / 4,
                    _ => random() as u8,
                };
            }
            let mut decoded = vec![0u8; source.len()];
            decode_attributes(&mut decoded, stride, &encode_attributes(&source, stride)).unwrap();
            assert_eq!(decoded, source, "stride {stride}, {count} elements");
        }
    }

    /// The worked example of a 4-bit group in the extension's specification.
    #[test]
    fn a_group_decodes_as_the_specification_shows() {
        let mut stream = vec![ATTRIBUTES_HEADER, 0b10];
        stream.extend([0x17, 0x5f, 0xf0, 0xbc, 0x77, 0xa9, 0x21, 0x00, 0x34, 0xb5]);
        // The other three bytes of the element don't change.
        stream.extend([0, 0, 0]);
        stream.extend([0; ATTRIBUTES_TAIL]);
        let mut decoded = [0u8; 16 * 4];
        decode_attributes(&mut decoded, 4, &stream).unwrap();
        let expected = [-1, -4, -3, 26, -91, 0, -6, 6, -4, -4, 5, -5, 1, -1, 0, 0];
        let mut value = 0u8;
        for (element, delta) in expected.into_iter().enumerate() {
            value = value.wrapping_add(delta as u8);
            assert_eq!(decoded[element * 4], value, "element {element}");
            assert_eq!(decoded[element * 4 + 1..element * 4 + 4], [0, 0, 0]);
        }
    }

    /// One triangle of each kind of code, each referring to the history the last ones left.
    fn triangle_stream() -> (Vec<u8>, Vec<[u32; 3]>) {
        let codes = [0xf0, 0x00, 0x01, 0x0f, 0x0d, 0x0e, 0xff, 0xfe, 0x00];
        // 0x0f: +7 from `last` = 0. 0xff: an entry of 0xf1, then +2 and -4 from `last` = 7.
        // 0xfe: an entry of 0, which restarts `next`.
        let extra = [0x0e, 0xf1, 0x04, 0x07, 0x00];
        let mut stream = vec![TRIANGLES_HEADER];
        stream.extend(codes);
        stream.extend(extra);
        stream.extend([0u8; TRIANGLES_TAIL]);
        let expected = vec![
            [0, 1, 2],
            [0, 2, 3],
            [0, 3, 2],
            [0, 2, 7],
            [0, 7, 6],
            [0, 6, 7],
            [9, 5, 7],
            [0, 1, 2],
            // Edge (0, 2), most recent, and the `next` that the restart left at 3.
            [0, 2, 3],
        ];
        (stream, expected)
    }

    #[test]
    fn each_triangle_code_reads_its_history() {
        let (stream, expected) = triangle_stream();
        let mut wide = vec![0u8; expected.len() * 12];
        decode_triangles(&mut wide, 4, &stream).unwrap();
        let wide: Vec<u32> = wide
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(wide, expected.concat());

        let mut narrow = vec![0u8; expected.len() * 6];
        decode_triangles(&mut narrow, 2, &stream).unwrap();
        let narrow: Vec<u32> = narrow
            .chunks_exact(2)
            .map(|b| u32::from(u16::from_le_bytes(b.try_into().unwrap())))
            .collect();
        assert_eq!(narrow, expected.concat());
    }

    #[test]
    fn indices_follow_two_baselines() {
        // +5 on baseline 0, +2 on baseline 1, -1 on baseline 0.
        let stream = [INDICES_HEADER, 0x14, 0x09, 0x02, 0, 0, 0, 0];
        let mut out = [0u8; 12];
        decode_indices(&mut out, 4, &stream).unwrap();
        let out: Vec<u32> = out
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(out, [5, 2, 4]);
    }

    #[test]
    fn damaged_streams_are_refused_not_panicked_on() {
        let source: Vec<u8> = (0..4096u32).map(|i| ((i * i) >> 3) as u8).collect();
        let attributes = encode_attributes(&source, 16);
        let mut out = vec![0u8; source.len()];
        for length in 0..attributes.len() {
            assert!(decode_attributes(&mut out, 16, &attributes[..length]).is_err());
        }
        let (triangles, expected) = triangle_stream();
        let mut out = vec![0u8; expected.len() * 12];
        for length in 0..triangles.len() {
            assert!(decode_triangles(&mut out, 4, &triangles[..length]).is_err());
        }
        let mut wrong = attributes.clone();
        wrong[0] = 0xa1;
        assert!(matches!(
            decode_attributes(&mut vec![0u8; source.len()], 16, &wrong),
            Err(DecodeError::Unsupported(_))
        ));
    }
}
