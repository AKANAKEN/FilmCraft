//! Hostile-input hammer: truncation, bit flips and extreme field values must never panic or hang.
//! Page CRCs are repaired before opening so the mutations reach the packet assembly code.

use super::*;

fn repair_crcs(d: &mut [u8]) {
    let mut at = 0;
    while let Some(p) = d.get(at..).and_then(|s| s.windows(4).position(|w| w == b"OggS")) {
        let start = at + p;
        at = start + 4;
        let Some(nseg) = d.get(start + 26).copied() else { break };
        let len = 27 + nseg as usize;
        let Some(lacing) = d.get(start + 27..start + len) else { break };
        let end = start + len + lacing.iter().map(|&b| b as usize).sum::<usize>();
        let Some(page) = d.get_mut(start..end) else { break };
        page[22..26].fill(0);
        let crc = crc32(page);
        page[22..26].copy_from_slice(&crc.to_le_bytes());
    }
}

fn exercise(d: &[u8]) {
    let mut fixed = d.to_vec();
    repair_crcs(&mut fixed);
    for data in [d, &fixed[..]] {
        let Ok(f) = open(data) else { continue };
        for (s, st) in f.streams.iter().enumerate() {
            let n = st.packets.len();
            for k in [0, n / 2, n.saturating_sub(1), n, usize::MAX] {
                let _ = f.read_packet(data, s, k);
            }
            let _ = OpusTiming::of(st, 0);
            let _ = OpusTiming::of(st, u32::MAX);
            for h in &st.headers {
                let _ = VorbisInfo::parse(h);
            }
            for p in &st.packets {
                let _ = opus_packet_samples(p.prefix, p.size);
            }
        }
    }
}

fn mixed_file() -> Vec<u8> {
    let mut f = Vec::new();
    // Vorbis: ident, comment, setup, then a packet spanning two pages
    let mut ident = b"\x01vorbis".to_vec();
    ident.extend_from_slice(&[0, 0, 0, 0, 2]);
    ident.extend_from_slice(&44100u32.to_le_bytes());
    ident.extend_from_slice(&[0; 14]);
    let (l, b) = lace(&[&ident], false);
    page(&mut f, 0x02, 0, 5, 0, &l, &b);
    let (l, b) = lace(&[b"\x03vorbis", b"\x05vorbis"], false);
    page(&mut f, 0, 0, 5, 1, &l, &b);
    let big = vec![7u8; 600];
    let (l, b) = lace(&[&big], true);
    page(&mut f, 0, 1000, 5, 2, &l, &b);
    let (l, b) = lace(&[&big[..10]], false);
    page(&mut f, 0x01 | 0x04, 2000, 5, 3, &l, &b);
    f
}

/// Regression: extreme granule positions overflowed `OpusTiming::of`'s i64 arithmetic.
#[test]
fn extreme_granules_do_not_overflow_opus_timing() {
    for g in [i64::MIN, i64::MIN + 1, -1, i64::MAX] {
        let mut f = Vec::new();
        let (l, b) = lace(&[&opus_head(0)], false);
        page(&mut f, 0x02, 0, 9, 0, &l, &b);
        let (l, b) = lace(&[b"OpusTags"], false);
        page(&mut f, 0, 0, 9, 1, &l, &b);
        for (seq, gran) in [(2, g), (3, i64::MAX), (4, g)] {
            let (l, b) = lace(&[&pkt(0, 40), &pkt(1, 40)], false);
            page(&mut f, if seq == 4 { 0x04 } else { 0 }, gran, 9, seq, &l, &b);
        }
        let o = open(&f[..]).unwrap();
        let _ = OpusTiming::of(&o.streams[0], u32::MAX);
    }
}

#[test]
fn hostile_input_never_panics() {
    let seeds = [opus_file(12, 312, 100), opus_file(2, 0, 0), mixed_file()];
    for (i, s) in seeds.iter().enumerate() {
        filmcraft_testkit::hostile::hammer(&format!("ogg seed {i}"), s, 40_000, exercise);
    }
}
