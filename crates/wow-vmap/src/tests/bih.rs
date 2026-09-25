use super::Rng;
use crate::io::Reader;
use crate::math::{AABox, Ray, Vector3};
use crate::{Bih, BihBuildError};

fn unit_box(x: f32) -> AABox {
    AABox::new(Vector3::new(x, 0.0, 0.0), Vector3::new(x + 1.0, 1.0, 1.0))
}

fn serialize(tree: &Bih) -> Vec<u8> {
    let mut buf = Vec::new();
    tree.write_to(&mut buf);
    buf
}

/// FNV-1a 64 over the serialized tree.
fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Random boxes; `flat` makes every box zero-height in Z, `dupes` repeats
/// boxes to force the "stuck" leaf path.
pub(crate) fn random_boxes(seed: u32, n: usize, range: u32, flat: bool, dupes: bool) -> Vec<AABox> {
    let mut rng = Rng(seed);
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let lo = Vector3::new(rng.coord(range), rng.coord(range), rng.coord(range));
        let size = Vector3::new(
            rng.coord(range / 8 + 1),
            rng.coord(range / 8 + 1),
            if flat { 0.0 } else { rng.coord(range / 8 + 1) },
        );
        let b = AABox::new(lo, lo + size);
        out.push(b);
        if dupes && out.len() < n && rng.next().is_multiple_of(3) {
            out.push(b);
        }
    }
    out
}

#[test]
fn empty_build_keeps_nan_bounds_and_dummy_leaf() {
    let mut t = Bih::default();
    t.build(&[], 3).unwrap();
    assert_eq!(t.tree(), &[3u32 << 30, 0, 0]);
    assert!(t.objects().is_empty());
    let bytes = serialize(&t);
    // bounds (NaN = 0x7FC00000), tree size 3, tree, object count 0
    assert_eq!(bytes.len(), 24 + 4 + 12 + 4);
    assert_eq!(&bytes[0..4], &0x7FC0_0000_u32.to_le_bytes());
    let back = Bih::read_from(&mut Reader::new(&bytes)).unwrap();
    assert_eq!(back.tree(), t.tree());
}

#[test]
fn two_boxes_hand_verified() {
    // Hand-traced through BIH::subdivide: axis X, split 1.5, clipL 1, clipR 2.
    let mut t = Bih::default();
    t.build(&[unit_box(0.0), unit_box(2.0)], 1).unwrap();
    assert_eq!(
        t.tree(),
        &[
            3,
            0x3F80_0000,
            0x4000_0000,
            0xC000_0000,
            1,
            0,
            0xC000_0001,
            1,
            0
        ]
    );
    assert_eq!(t.objects(), &[0, 1]);
    assert_eq!(t.bounds().low(), Vector3::new(0.0, 0.0, 0.0));
    assert_eq!(t.bounds().high(), Vector3::new(3.0, 1.0, 1.0));
}

#[test]
fn leaf_when_under_leaf_size() {
    let mut t = Bih::default();
    t.build(&[unit_box(0.0), unit_box(5.0), unit_box(9.0)], 3)
        .unwrap();
    assert_eq!(t.tree(), &[3u32 << 30, 3, 0]);
    assert_eq!(t.objects(), &[0, 1, 2]);
}

#[test]
fn identical_boxes_terminate_as_stuck_leaf() {
    let boxes = vec![unit_box(1.0); 10];
    let mut t = Bih::default();
    t.build(&boxes, 1).unwrap();
    assert_eq!(t.prim_count(), 10);
    // every object ends in some leaf exactly once
    let mut seen = t.objects().to_vec();
    seen.sort_unstable();
    assert_eq!(seen, (0..10).collect::<Vec<u32>>());
}

#[test]
fn invalid_bounds_raise_logic_error() {
    // hi < lo gives negative extents
    let bad = AABox::new(Vector3::new(1.0, 1.0, 1.0), Vector3::new(0.0, 0.0, 0.0));
    let mut t = Bih::default();
    assert_eq!(
        t.build(&[bad, bad, bad, bad], 1),
        Err(BihBuildError::NegativeNodeExtents)
    );
}

/// Golden outputs produced by the C++ `BIH::build` + `BIH::writeToFile`
/// (`TrinityCore` `TDB343.24081`, g++ -O2) for the same inputs; see
/// `bih_matches_cpp_reference` for the generator harness.
#[test]
#[allow(clippy::type_complexity)]
fn golden_hashes_from_cpp() {
    let cases: [(u32, usize, u32, bool, bool, u32, usize, u64); 6] = [
        // seed, n, range, flat, dupes, leaf, serialized len, fnv1a
        (1, 50, 100, false, false, 3, GOLDEN[0].0, GOLDEN[0].1),
        (2, 200, 1000, false, false, 3, GOLDEN[1].0, GOLDEN[1].1),
        (3, 64, 50, true, false, 1, GOLDEN[2].0, GOLDEN[2].1),
        (4, 300, 20, false, true, 3, GOLDEN[3].0, GOLDEN[3].1),
        (5, 1000, 17_000, true, false, 3, GOLDEN[4].0, GOLDEN[4].1),
        (6, 17, 5, false, true, 1, GOLDEN[5].0, GOLDEN[5].1),
    ];
    for (seed, n, range, flat, dupes, leaf, len, hash) in cases {
        let boxes = random_boxes(seed, n, range, flat, dupes);
        let mut t = Bih::default();
        t.build(&boxes, leaf).unwrap();
        let bytes = serialize(&t);
        assert_eq!(
            (bytes.len(), fnv1a(&bytes)),
            (len, hash),
            "case seed {seed}"
        );
        // round trip
        let back = Bih::read_from(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(serialize(&back), bytes);
    }
}

const GOLDEN: [(usize, u64); 6] = [
    (844, 0x8ed5_b016_b67a_8dff),
    (3520, 0x8d35_3f12_4e92_17dd),
    (2292, 0x0947_1225_8525_52de),
    (5324, 0xf5d4_5ecc_f9af_d3c7),
    (16476, 0x067d_2168_906d_6fc2),
    (496, 0x3eda_dfbd_19e3_6441),
];

/// Compares against the C++ harness when `VMAP_CPP_BIH` points to it
/// (reads `leaf n` + hex box bits on stdin, writes `BIH::writeToFile`).
#[test]
#[ignore = "needs the C++ reference harness (VMAP_CPP_BIH)"]
fn bih_matches_cpp_reference() {
    use std::fmt::Write as _;
    use std::io::Write;
    let harness = std::env::var("VMAP_CPP_BIH").expect("VMAP_CPP_BIH");
    let mut cases = vec![
        (1, 50, 100, false, false, 3),
        (2, 200, 1000, false, false, 3),
        (3, 64, 50, true, false, 1),
        (4, 300, 20, false, true, 3),
        (5, 1000, 17_000, true, false, 3),
        (6, 17, 5, false, true, 1),
    ];
    for seed in 100..400u32 {
        cases.push((
            seed,
            (seed as usize * 7) % 500 + 1,
            [3, 30, 300, 5000][seed as usize % 4],
            seed % 5 == 0,
            seed % 3 == 0,
            [1, 3][seed as usize % 2],
        ));
    }
    for (seed, n, range, flat, dupes, leaf) in cases {
        let boxes = random_boxes(seed, n, range, flat, dupes);
        let mut input = format!("{leaf} {}\n", boxes.len());
        for b in &boxes {
            for v in [b.low(), b.high()] {
                let _ = write!(
                    input,
                    "{:x} {:x} {:x} ",
                    v.x.to_bits(),
                    v.y.to_bits(),
                    v.z.to_bits()
                );
            }
            input.push('\n');
        }
        let mut child = std::process::Command::new(&harness)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success());
        let mut t = Bih::default();
        t.build(&boxes, leaf).unwrap();
        let bytes = serialize(&t);
        println!("seed {seed}: len {} fnv {:#x}", bytes.len(), fnv1a(&bytes));
        assert_eq!(bytes, out.stdout, "seed {seed}");
    }
}

#[test]
fn intersect_point_and_ray_find_boxes() {
    let boxes = random_boxes(9, 200, 100, false, false);
    let mut t = Bih::default();
    t.build(&boxes, 3).unwrap();

    // point queries must report every box containing the point
    let mut rng = Rng(77);
    for _ in 0..200 {
        let p = Vector3::new(rng.coord(110), rng.coord(110), rng.coord(110));
        let mut found = Vec::new();
        t.intersect_point(p, &mut |_, idx| found.push(idx));
        for (i, b) in boxes.iter().enumerate() {
            if b.contains(p) {
                assert!(found.contains(&(i as u32)), "box {i} missing for {p:?}");
            }
        }
    }

    // a ray down the Z axis through a box center must visit that box
    for (i, b) in boxes.iter().enumerate().take(50) {
        let c = (b.low() + b.high()) * 0.5;
        let ray = Ray::from_origin_and_direction(
            Vector3::new(c.x, c.y, 500.0),
            Vector3::new(0.0, 0.0, -1.0),
        );
        let mut visited = Vec::new();
        let mut max_dist = 1000.0;
        t.intersect_ray(
            &ray,
            &mut |_, idx, _, _| {
                visited.push(idx);
                false
            },
            &mut max_dist,
            false,
        );
        assert!(visited.contains(&(i as u32)), "box {i} not visited");
    }
}
