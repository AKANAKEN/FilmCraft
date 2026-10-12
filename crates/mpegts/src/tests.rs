//! Synthetic transport and program streams: a small muxer writes known access units, the
//! demuxer must give them back exactly (bytes, timestamps, flags); truncation and mutation never
//! panic.

use super::*;

fn ts_bytes(prefix: u8, v: i64) -> [u8; 5] {
    let v = v & ((1 << 33) - 1);
    [(prefix << 4) | (((v >> 30) & 7) as u8) << 1 | 1, (v >> 22) as u8, (((v >> 15) & 0x7F) as u8) << 1 | 1, (v >> 7) as u8, ((v & 0x7F) as u8) << 1 | 1]
}

fn pes(stream_id: u8, pts: Option<i64>, dts: Option<i64>, payload: &[u8], bounded: bool) -> Vec<u8> {
    let mut h = Vec::new();
    match (pts, dts) {
        (Some(p), Some(d)) => {
            h.extend(ts_bytes(3, p));
            h.extend(ts_bytes(1, d));
        }
        (Some(p), None) => h.extend(ts_bytes(2, p)),
        _ => {}
    }
    let flags = match (pts, dts) {
        (Some(_), Some(_)) => 0xC0,
        (Some(_), None) => 0x80,
        _ => 0,
    };
    let len = if bounded { 3 + h.len() + payload.len() } else { 0 };
    let mut v = vec![0, 0, 1, stream_id, (len >> 8) as u8, len as u8, 0x80, flags, h.len() as u8];
    v.extend(h);
    v.extend_from_slice(payload);
    v
}

/// A tiny TS muxer: PAT, PMT and PES packets cut into 188-byte packets (192 with `m2ts`).
struct TsMux {
    out: Vec<u8>,
    cc: std::collections::HashMap<u16, u8>,
    m2ts: bool,
}

impl TsMux {
    fn new(m2ts: bool) -> Self {
        Self { out: Vec::new(), cc: Default::default(), m2ts }
    }
    fn packet(&mut self, pid: u16, pusi: bool, payload: &[u8]) -> usize {
        let cc = self.cc.entry(pid).or_insert(0);
        let mut p = vec![0x47, (pusi as u8) << 6 | (pid >> 8) as u8, pid as u8];
        let n = payload.len().min(184);
        if n < 184 {
            // adaptation field stuffing
            let al = 183 - n;
            p.push(0x30 | *cc);
            p.push(al as u8);
            if al > 0 {
                p.push(0);
                p.extend(std::iter::repeat_n(0xFF, al - 1));
            }
        } else {
            p.push(0x10 | *cc);
        }
        *cc = (*cc + 1) & 15;
        p.extend_from_slice(&payload[..n]);
        assert_eq!(p.len(), 188);
        if self.m2ts {
            self.out.extend_from_slice(&[0, 0, 0, 0]);
        }
        self.out.extend(p);
        n
    }
    fn section(&mut self, pid: u16, body: &[u8]) {
        let mut s = body.to_vec();
        let crc = crate::ts::crc32_mpeg(&s);
        s.extend(crc.to_be_bytes());
        let mut payload = vec![0];
        payload.extend(s);
        self.packet(pid, true, &payload);
    }
    fn psi(&mut self, streams: &[(u8, u16)]) {
        // PAT: program 1 → PMT PID 0x100
        let pat = [0x00, 0xB0, 13, 0, 1, 0xC1, 0, 0, 0, 1, 0xE1, 0x00];
        self.section(0, &pat);
        let mut pmt = vec![0x02, 0xB0, 0, 0, 1, 0xC1, 0, 0, 0xE1, 0x00, 0xF0, 0];
        for &(st, pid) in streams {
            pmt.extend([st, 0xE0 | (pid >> 8) as u8, pid as u8, 0xF0, 0]);
        }
        let len = pmt.len() - 3 + 4;
        pmt[1] = 0xB0 | (len >> 8) as u8;
        pmt[2] = len as u8;
        self.section(0x100, &pmt);
    }
    fn pes(&mut self, pid: u16, data: &[u8]) {
        let mut rest = data;
        let mut first = true;
        while !rest.is_empty() {
            let n = self.packet(pid, first, rest);
            rest = &rest[n..];
            first = false;
        }
    }
}

/// An MPEG-2 coded frame: sequence header (on I), GOP, picture header, picture coding
/// extension and a slice of filler; or a field pair.
fn mpeg2_frame(coding_type: u8, tref: u16, fields: bool, filler: usize) -> Vec<u8> {
    let mut v = Vec::new();
    if coding_type == 1 {
        v.extend([0, 0, 1, 0xB3, 0x14, 0x00, 0xF0, 0x13, 0xFF, 0xFF, 0xE0, 0x18]);
        v.extend([0, 0, 1, 0xB8, 0, 8, 0, 0x40]);
    }
    let pics: &[u8] = if fields { &[1, 2] } else { &[3] };
    for &s in pics {
        v.extend([0, 0, 1, 0x00, (tref >> 2) as u8, ((tref & 3) << 6) as u8 | coding_type << 3, 0xFF, 0xF8]);
        v.extend([0, 0, 1, 0xB5, 0x8F, 0xFF, 0xF0 | s, 0x80, 0x80]);
        v.extend([0, 0, 1, 0x01]);
        v.extend((0..filler).map(|i| (i % 200) as u8 | 0x10));
    }
    v
}

/// MPEG-1 layer II frame, 48 kHz 192 kb/s (576 bytes).
fn mp2_frame(k: u8) -> Vec<u8> {
    let mut v = vec![0xFF, 0xFD, 0xA4, 0x00];
    v.extend((0..572).map(|i| (i as u8) ^ k));
    v
}

#[test]
fn transport_stream_units_round_trip() {
    for m2ts in [false, true] {
        let mut mux = TsMux::new(m2ts);
        mux.psi(&[(0x02, 0x1011), (0x03, 0x1100)]);
        // decode order I0 P3 B1 B2 with PTS = display time, DTS on anchors
        let frames = [(1u8, 0u16, false), (2, 3, false), (3, 1, true), (3, 2, false)];
        let mut expect_video = Vec::new();
        for (k, &(t, tref, fields)) in frames.iter().enumerate() {
            let f = mpeg2_frame(t, tref, fields, 300 + 97 * k);
            let pts = 90_000 + tref as i64 * 3600;
            let dts = (t != 3).then_some(90_000 - 3600 + k as i64 * 3600);
            mux.pes(0x1011, &pes(0xE0, Some(pts), dts, &f, false));
            expect_video.push((f, pts, t));
            // two audio frames per PES, the second without its own timestamp
            let a: Vec<u8> = [mp2_frame(2 * k as u8), mp2_frame(2 * k as u8 + 1)].concat();
            mux.pes(0x1100, &pes(0xC0, Some(90_000 + 2160 * 2 * k as i64), None, &a, true));
        }
        let data = mux.out;
        let f = open(&data).unwrap();
        assert_eq!(f.format, Format::Ts { packet_size: if m2ts { 192 } else { 188 }, sync_offset: if m2ts { 4 } else { 0 } });
        assert!(f.warnings.is_empty(), "{:?}", f.warnings);
        assert_eq!(f.program, Some(1));
        let v = f.find(Kind::Video, |_| true).unwrap();
        let s = &f.streams[v];
        assert_eq!(s.codec, Codec::Mpeg2Video);
        assert_eq!(s.units.len(), 4);
        for (i, (bytes, pts, t)) in expect_video.iter().enumerate() {
            let u = &s.units[i];
            assert_eq!(f.read_unit(&data, v, i).unwrap(), *bytes, "unit {i}");
            assert_eq!(u.pts, Some(*pts));
            assert_eq!(u.key, *t == 1);
            assert_eq!(u.disposable, *t == 3);
            assert_eq!(u.picture.unwrap().coding_type, *t);
        }
        assert_eq!(s.units[2].picture.unwrap().structure, 1, "field pair in one unit");
        assert!(s.units[0].picture.unwrap().sequence_header && s.units[0].picture.unwrap().gop);
        let a = f.find(Kind::Audio, |c| *c == Codec::MpegAudio).unwrap();
        let s = &f.streams[a];
        assert_eq!(s.units.len(), 8);
        for (i, u) in s.units.iter().enumerate() {
            assert_eq!(f.read_unit(&data, a, i).unwrap(), mp2_frame(i as u8));
            assert_eq!(u.pts, (i % 2 == 0).then_some(90_000 + 2160 * i as i64));
        }
    }
}

#[test]
fn program_stream_units_round_trip() {
    // packs of ≤ 2048 bytes: frames are split across PES packets
    let frames: Vec<Vec<u8>> = (0..5).map(|k| mpeg2_frame(if k == 0 { 1 } else { 2 }, k as u16, k == 3, 2500 + 300 * k)).collect();
    let es: Vec<u8> = frames.concat();
    let mut starts = Vec::new();
    let mut acc = 0;
    for f in &frames {
        starts.push(acc);
        acc += f.len();
    }
    let mut out = Vec::new();
    let mut p = 0;
    let mut a = 0u8;
    while p < es.len() {
        out.extend([0, 0, 1, 0xBA, 0x44, 0, 4, 0, 4, 1, 1, 0x89, 0xC3, 0xF8]);
        let n = (es.len() - p).min(2000);
        // a timestamp when a frame starts in this packet
        let k = starts.iter().position(|&s| s >= p && s < p + n);
        out.extend(pes(0xE0, k.map(|k| 3600 * k as i64), None, &es[p..p + n], true));
        p += n;
        // AC-3 in private stream 1 (sub-stream 0x80, 4-byte header), 448 kb/s frames
        let mut ac3 = vec![0x80, 1, 0, 1, 0x0B, 0x77, 0, 0, 30, 0x40];
        ac3.extend(std::iter::repeat_n(a, 1792 - 6));
        a += 1;
        out.extend(pes(0xBD, Some(1000 + 2880 * a as i64), None, &ac3, true));
    }
    out.extend([0, 0, 1, 0xB9]);
    let f = open(&out).unwrap();
    assert_eq!(f.format, Format::Ps { mpeg1: false });
    let v = f.find(Kind::Video, |_| true).unwrap();
    assert_eq!(f.streams[v].units.len(), 5);
    for (i, fr) in frames.iter().enumerate() {
        assert_eq!(&f.read_unit(&out, v, i).unwrap(), fr);
        assert_eq!(f.streams[v].units[i].pts, Some(3600 * i as i64));
    }
    let a = f.find(Kind::Audio, |c| *c == Codec::Ac3).unwrap();
    assert_eq!(f.streams[a].id, StreamId::Ps { stream_id: 0xBD, sub_id: Some(0x80) });
    assert_eq!(f.streams[a].units.len(), a_count(&out));
    let u0 = f.read_unit(&out, a, 0).unwrap();
    assert_eq!(&u0[..2], &[0x0B, 0x77]);
    assert_eq!(u0.len(), 1792);
}

fn a_count(ps: &[u8]) -> usize {
    ps.windows(4).filter(|w| *w == [0, 0, 1, 0xBD]).count()
}

#[test]
fn sniffing() {
    assert_eq!(sniff(&[0, 0, 1, 0xBA, 0x44]), Some(Format::Ps { mpeg1: false }));
    assert_eq!(sniff(&[0, 0, 1, 0xBA, 0x21]), Some(Format::Ps { mpeg1: true }));
    assert_eq!(sniff(&[0, 0, 1, 0xB3, 0x44]), None);
    let mut ts = vec![0u8; 188 * 6];
    for k in 0..6 {
        ts[k * 188] = 0x47;
    }
    assert!(matches!(sniff(&ts), Some(Format::Ts { packet_size: 188, .. })));
}

#[test]
fn truncation_and_mutation_never_panic() {
    let mut mux = TsMux::new(false);
    mux.psi(&[(0x02, 0x1011), (0x0F, 0x1100), (0x1B, 0x1200)]);
    for k in 0..6u16 {
        mux.pes(0x1011, &pes(0xE0, Some(3600 * k as i64), None, &mpeg2_frame(if k == 0 { 1 } else { 3 }, k, k % 2 == 1, 400), false));
        let mut adts = vec![0xFF, 0xF1, 0x50, 0x80, 0x0C, 0x9F, 0xFC];
        adts.extend([0x21u8; 93]);
        mux.pes(0x1100, &pes(0xC0, Some(1920 * k as i64), None, &adts, true));
        mux.pes(0x1200, &pes(0xE1, Some(3600 * k as i64), None, &[0, 0, 0, 1, 0x09, 0xF0, 0, 0, 1, 0x65, 0x88, 1, 2, 3], false));
    }
    let data = mux.out;
    let full = open(&data).unwrap();
    assert_eq!(full.streams.len(), 3);
    assert_eq!(full.streams[1].units.len(), 6, "ADTS frames");
    assert!(full.streams[2].units.iter().all(|u| u.key), "IDR access units");
    for cut in (0..data.len()).step_by(37) {
        if let Ok(f) = open(&data[..cut].to_vec()) {
            for (s, st) in f.streams.iter().enumerate() {
                for i in 0..st.units.len() {
                    let _ = f.read_unit(&data[..cut].to_vec(), s, i);
                }
            }
        }
    }
    let mut x = 0x9E37_79B9u32;
    for _ in 0..400 {
        let mut m = data.clone();
        for _ in 0..8 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let i = x as usize % m.len();
            m[i] = (x >> 8) as u8;
        }
        if let Ok(f) = open(&m) {
            for (s, st) in f.streams.iter().enumerate() {
                for i in 0..st.units.len() {
                    let _ = f.read_unit(&m, s, i);
                }
            }
        }
    }
}

#[test]
fn timestamps_unwrap_across_the_33_bit_wrap() {
    let mut u = Unwrap::default();
    let near = (1i64 << 33) - 3000;
    assert_eq!(u.unwrap(near), near);
    assert_eq!(u.unwrap(600), (1 << 33) + 600);
    // a B picture timestamp from just before the wrap
    assert_eq!(u.unwrap(near + 1000), near + 1000);
    assert_eq!(u.unwrap(4200), (1 << 33) + 4200);
}

/// Audio-only TS: `n` PES packets of two MPEG-1 layer II frames each (7 transport packets apiece).
fn audio_ts(n: u8) -> Vec<u8> {
    let mut mux = TsMux::new(false);
    mux.psi(&[(0x03, 0x1100)]);
    for k in 0..n {
        let a: Vec<u8> = [mp2_frame(2 * k), mp2_frame(2 * k + 1)].concat();
        mux.pes(0x1100, &pes(0xC0, Some(90_000 + 4320 * k as i64), None, &a, true));
    }
    mux.out
}

/// Byte offset of the `n`-th (0-based) packet of `pid` that has no unit start.
fn nth_continuation(data: &[u8], pid: u16, n: usize) -> usize {
    (0..data.len() / 188)
        .map(|i| i * 188)
        .filter(|&o| data[o] == 0x47 && (((data[o + 1] & 0x1F) as u16) << 8 | data[o + 2] as u16) == pid && data[o + 1] & 0x40 == 0)
        .nth(n)
        .expect("packet exists")
}

fn audio_bytes(f: &File, data: &[u8]) -> Vec<Vec<u8>> {
    let a = f.find(Kind::Audio, |_| true).unwrap();
    (0..f.streams[a].units.len()).map(|i| f.read_unit(data, a, i).unwrap()).collect()
}

#[test]
fn packet_with_transport_error_indicator_is_dropped() {
    let clean = audio_ts(6);
    let base = open(&clean).unwrap();
    assert!(base.warnings.is_empty(), "{:?}", base.warnings);
    let mut data = clean.clone();
    let o = nth_continuation(&data, 0x1100, 10);
    data[o + 1] |= 0x80;
    // a corrupted payload must not reach the splitter either
    data[o + 20] ^= 0xFF;
    let f = open(&data).unwrap();
    assert!(f.warnings.iter().any(|w| w.contains("1 packet(s) with the transport error indicator")), "{:?}", f.warnings);
    let a = f.find(Kind::Audio, |_| true).unwrap();
    let total = |f: &File| f.streams[a].units.iter().map(|u| u.size as u64).sum::<u64>();
    assert!(total(&f) < total(&base), "the 184 payload bytes of the flagged packet are gone");
    for unit in audio_bytes(&f, &data) {
        assert!(!unit.is_empty());
    }
}

#[test]
fn scrambled_packet_is_skipped_with_a_warning() {
    let clean = audio_ts(6);
    let base = open(&clean).unwrap();
    let mut data = clean.clone();
    let o = nth_continuation(&data, 0x1100, 10);
    data[o + 3] |= 0x80; // transport_scrambling_control = 2 (even key)
    let f = open(&data).unwrap();
    assert!(f.warnings.iter().any(|w| w.contains("1 scrambled packet(s) skipped")), "{:?}", f.warnings);
    assert!(!f.warnings.iter().any(|w| w.contains("continuity")), "{:?}", f.warnings);
    let a = f.find(Kind::Audio, |_| true).unwrap();
    let total = |f: &File| f.streams[a].units.iter().map(|u| u.size as u64).sum::<u64>();
    assert!(total(&f) < total(&base));
}

#[test]
fn duplicate_packet_is_dropped_but_a_different_payload_with_the_same_counter_is_kept() {
    let clean = audio_ts(6);
    let base = open(&clean).unwrap();
    let expect = audio_bytes(&base, &clean);
    let o = nth_continuation(&clean, 0x1100, 10);
    // exact duplicate right behind the original
    let mut dup = clean.clone();
    let copy = dup[o..o + 188].to_vec();
    dup.splice(o + 188..o + 188, copy);
    let f = open(&dup).unwrap();
    assert!(f.warnings.iter().any(|w| w.contains("1 duplicate packet(s) dropped")), "{:?}", f.warnings);
    assert!(!f.warnings.iter().any(|w| w.contains("continuity")), "{:?}", f.warnings);
    assert_eq!(audio_bytes(&f, &dup), expect, "the repeated payload is not fed twice");
    // same counter but other content: not a duplicate, kept
    let mut other = clean.clone();
    let mut copy = other[o..o + 188].to_vec();
    copy[187] ^= 0x55;
    other.splice(o + 188..o + 188, copy);
    let f = open(&other).unwrap();
    assert!(!f.warnings.iter().any(|w| w.contains("duplicate")), "{:?}", f.warnings);
    let a = f.find(Kind::Audio, |_| true).unwrap();
    let total = |f: &File| f.streams[a].units.iter().map(|u| u.size as u64).sum::<u64>();
    assert!(total(&f) > total(&base), "the differing packet's payload is kept");
}

/// A PMT section for program 1 with the given version / current_next / streams (valid CRC).
fn pmt_section(version: u8, current_next: bool, streams: &[(u8, u16)]) -> Vec<u8> {
    let mut pmt = vec![0x02, 0xB0, 0, 0, 1, 0xC0 | (version << 1) | current_next as u8, 0, 0, 0xE1, 0x00, 0xF0, 0];
    for &(st, pid) in streams {
        pmt.extend([st, 0xE0 | (pid >> 8) as u8, pid as u8, 0xF0, 0]);
    }
    let len = pmt.len() - 3 + 4;
    pmt[1] = 0xB0 | (len >> 8) as u8;
    pmt[2] = len as u8;
    pmt
}

#[test]
fn first_pmt_wins_later_changes_are_ignored_with_one_warning() {
    let mut mux = TsMux::new(false);
    mux.psi(&[(0x03, 0x1100)]);
    let audio = |mux: &mut TsMux, k: u8| {
        let a: Vec<u8> = [mp2_frame(2 * k), mp2_frame(2 * k + 1)].concat();
        mux.pes(0x1100, &pes(0xC0, Some(90_000 + 4320 * k as i64), None, &a, true));
    };
    audio(&mut mux, 0);
    // version 1: the PID becomes AAC and a second stream appears; version 2: PID removed
    mux.section(0x100, &pmt_section(1, true, &[(0x0F, 0x1100), (0x03, 0x1200)]));
    audio(&mut mux, 1);
    mux.section(0x100, &pmt_section(2, true, &[(0x03, 0x1200)]));
    mux.section(0x100, &pmt_section(2, true, &[(0x03, 0x1200)]));
    audio(&mut mux, 2);
    let f = open(&mux.out).unwrap();
    assert_eq!(f.streams.len(), 1, "{:?}", f.streams.iter().map(|s| s.id).collect::<Vec<_>>());
    assert_eq!(f.streams[0].id, StreamId::Pid(0x1100));
    assert_eq!(f.streams[0].codec, Codec::MpegAudio);
    assert_eq!(f.streams[0].units.len(), 6, "audio kept flowing through the first PMT's splitter");
    assert_eq!(f.warnings.iter().filter(|w| w.contains("program map table changed")).count(), 1, "{:?}", f.warnings);
}

#[test]
fn pmt_not_yet_applicable_is_ignored() {
    let mut mux = TsMux::new(false);
    let pat = [0x00, 0xB0, 13, 0, 1, 0xC1, 0, 0, 0, 1, 0xE1, 0x00];
    mux.section(0, &pat);
    // current_next_indicator 0: describes a future table, must not be used
    mux.section(0x100, &pmt_section(3, false, &[(0x03, 0x1200)]));
    mux.section(0x100, &pmt_section(0, true, &[(0x03, 0x1100)]));
    let a: Vec<u8> = [mp2_frame(0), mp2_frame(1)].concat();
    mux.pes(0x1100, &pes(0xC0, Some(90_000), None, &a, true));
    let f = open(&mux.out).unwrap();
    assert_eq!(f.streams.len(), 1);
    assert_eq!(f.streams[0].id, StreamId::Pid(0x1100));
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
}
