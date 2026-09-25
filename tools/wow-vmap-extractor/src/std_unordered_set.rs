//! Emulation of the iteration order of libstdc++ `std::unordered_set<uint16>`.
//!
//! `WMODoodadData::References` (`wmo.h`) is a `std::unordered_set<uint16>` and
//! `Doodad::ExtractSet` (`model.cpp`) iterates it: that order decides the order of the
//! doodad spawns written to `dir_bin` and the `doodadId` passed to
//! `GenerateUniqueObjectId`, i.e. the unique ids of every WMO doodad spawn. To produce
//! byte-identical output to the C++ tool built with GCC/libstdc++ (the Linux build), the
//! container is emulated exactly:
//!
//! - `std::hash<uint16>` is the identity, bucket = `value % bucket_count`;
//! - `_M_insert_bucket_begin`: a node whose bucket is empty goes to the front of the
//!   global list, otherwise it is linked right after the bucket's "before" node;
//! - `_M_rehash_aux(n, true_type)` relinks the old list in order with the same rule;
//! - `_Prime_rehash_policy` (max load factor 1.0, growth factor 2) grows a
//!   default-constructed set through the bucket counts in [`BUCKET_GROWTH`].
//!
//! `BUCKET_GROWTH` and the unit test orders were produced by an oracle program compiled
//! with g++ 13.3 (libstdc++), inserting into a default-constructed
//! `std::unordered_set<uint16_t>` and printing `bucket_count()` / iteration order.
//! The MSVC implementation iterates differently; the Windows build of the C++ tool does
//! not produce the same unique ids for WMO doodads.

/// `(element count that triggers the rehash, new bucket count)`: a rehash happens when
/// inserting would make the size exceed the current bucket count (`_M_next_resize`).
const BUCKET_GROWTH: [usize; 13] = [
    13, 29, 59, 127, 257, 541, 1109, 2357, 5087, 10273, 20753, 42043, 85229,
];

#[derive(Debug, Clone)]
struct Node {
    value: u16,
    next: Option<usize>,
}

/// "before" pointer of a bucket: `_M_before_begin` or a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Before {
    Begin,
    Node(usize),
}

/// `std::unordered_set<uint16>` with libstdc++ iteration order.
#[derive(Debug, Clone)]
pub struct StdUnorderedSetU16 {
    nodes: Vec<Node>,
    head: Option<usize>,
    buckets: Vec<Option<Before>>,
    present: std::collections::HashSet<u16>,
}

impl Default for StdUnorderedSetU16 {
    fn default() -> Self {
        Self::new()
    }
}

impl StdUnorderedSetU16 {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            head: None,
            // default-constructed: single bucket, `_M_next_resize == 0`
            buckets: vec![None],
            present: std::collections::HashSet::new(),
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    fn next_resize(&self) -> usize {
        // `_M_next_resize` is 0 until the first allocation, then equals the bucket count
        // (max load factor 1.0).
        if self.nodes.is_empty() && self.buckets.len() == 1 {
            0
        } else {
            self.buckets.len()
        }
    }

    /// `insert(value)` (`_M_insert_unique`).
    pub fn insert(&mut self, value: u16) -> bool {
        if self.present.contains(&value) {
            return false;
        }

        // _M_need_rehash(bucket_count, element_count, 1)
        if self.nodes.len() + 1 > self.next_resize() {
            let new_count = BUCKET_GROWTH
                .iter()
                .copied()
                .find(|&b| b > self.buckets.len() && b > self.nodes.len())
                .expect("uint16 set cannot outgrow the growth table");
            self.rehash(new_count);
        }

        let idx = self.nodes.len();
        self.nodes.push(Node { value, next: None });
        self.present.insert(value);
        let bkt = value as usize % self.buckets.len();
        self.insert_bucket_begin(bkt, idx);
        true
    }

    fn bucket_of(&self, node: usize) -> usize {
        self.nodes[node].value as usize % self.buckets.len()
    }

    fn before_next(&self, before: Before) -> Option<usize> {
        match before {
            Before::Begin => self.head,
            Before::Node(n) => self.nodes[n].next,
        }
    }

    fn set_before_next(&mut self, before: Before, next: Option<usize>) {
        match before {
            Before::Begin => self.head = next,
            Before::Node(n) => self.nodes[n].next = next,
        }
    }

    /// `_M_insert_bucket_begin`.
    fn insert_bucket_begin(&mut self, bkt: usize, node: usize) {
        if let Some(before) = self.buckets[bkt] {
            self.nodes[node].next = self.before_next(before);
            self.set_before_next(before, Some(node));
        } else {
            self.nodes[node].next = self.head;
            self.head = Some(node);
            if let Some(next) = self.nodes[node].next {
                let next_bkt = self.bucket_of(next);
                self.buckets[next_bkt] = Some(Before::Node(node));
            }
            self.buckets[bkt] = Some(Before::Begin);
        }
    }

    /// `_M_rehash_aux(bkt_count, true_type)`.
    fn rehash(&mut self, bkt_count: usize) {
        let mut new_buckets: Vec<Option<Before>> = vec![None; bkt_count];
        let mut p = self.head;
        self.head = None;
        let mut bbegin_bkt = 0usize;
        while let Some(cur) = p {
            let next = self.nodes[cur].next;
            let bkt = self.nodes[cur].value as usize % bkt_count;
            if let Some(before) = new_buckets[bkt] {
                let after = self.before_next(before);
                self.nodes[cur].next = after;
                self.set_before_next(before, Some(cur));
            } else {
                self.nodes[cur].next = self.head;
                self.head = Some(cur);
                new_buckets[bkt] = Some(Before::Begin);
                if self.nodes[cur].next.is_some() {
                    new_buckets[bbegin_bkt] = Some(Before::Node(cur));
                }
                bbegin_bkt = bkt;
            }
            p = next;
        }
        self.buckets = new_buckets;
    }

    /// Values in container iteration order.
    pub fn iter(&self) -> impl Iterator<Item = u16> + '_ {
        let mut p = self.head;
        std::iter::from_fn(move || {
            let cur = p?;
            p = self.nodes[cur].next;
            Some(self.nodes[cur].value)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_libstdcxx_order_small() {
        let seq = [
            5u16, 18, 31, 0, 13, 7, 44, 5, 100, 26, 2, 57, 70, 3, 1, 9, 4, 60, 88, 29, 12, 65535,
            300, 17, 1000, 42, 13, 11, 22, 33,
        ];
        let mut s = StdUnorderedSetU16::new();
        for v in seq {
            s.insert(v);
        }
        let got: Vec<u16> = s.iter().collect();
        let expected = [
            22, 11, 1000, 17, 300, 65535, 33, 4, 9, 88, 1, 5, 18, 44, 57, 12, 70, 29, 0, 26, 7, 42,
            13, 100, 60, 31, 2, 3,
        ];
        assert_eq!(got, expected);
    }

    #[test]
    fn matches_libstdcxx_order_lcg() {
        let mut s = StdUnorderedSetU16::new();
        let mut x: u32 = 12345;
        for _ in 0..200 {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            s.insert(((x >> 16) % 700) as u16);
        }
        let got: Vec<u16> = s.iter().collect();
        let expected: [u16; 170] = [
            385, 156, 261, 638, 124, 105, 422, 107, 236, 300, 557, 574, 191, 526, 295, 445, 125,
            407, 224, 263, 463, 233, 310, 250, 507, 667, 153, 280, 672, 158, 285, 512, 255, 272,
            18, 221, 147, 492, 365, 303, 607, 99, 613, 359, 379, 506, 449, 192, 251, 311, 650, 142,
            443, 514, 37, 548, 34, 291, 627, 336, 10, 264, 98, 372, 629, 248, 121, 502, 229, 577,
            453, 196, 695, 9, 184, 698, 444, 94, 351, 97, 56, 183, 177, 691, 416, 159, 673, 546,
            458, 201, 671, 157, 544, 30, 87, 464, 637, 380, 161, 675, 50, 168, 371, 654, 397, 82,
            596, 90, 149, 166, 680, 495, 106, 620, 296, 474, 565, 308, 456, 202, 519, 138, 178,
            452, 195, 322, 197, 64, 578, 618, 104, 600, 533, 409, 666, 152, 398, 401, 658, 144,
            489, 41, 298, 480, 223, 645, 388, 472, 599, 584, 330, 211, 513, 256, 2, 624, 360, 162,
            665, 151, 258, 205, 462, 254, 511, 389, 132, 609, 569, 312,
        ];
        assert_eq!(got, expected);
    }

    #[test]
    fn full_range_fits() {
        let mut s = StdUnorderedSetU16::new();
        for v in 0..=u16::MAX {
            s.insert(v);
        }
        assert_eq!(s.len(), 65536);
        assert_eq!(s.iter().count(), 65536);
    }
}
