//! Bounding Interval Hierarchy — port of `BIH` from
//! `src/common/Collision/BoundingIntervalHierarchy.{h,cpp}`.
//!
//! The builder reproduces `BIH::build`/`buildHierarchy`/`subdivide` exactly
//! (same split heuristic, same partition swaps, same `f32` bit patterns), so
//! the serialized tree is byte-identical to the C++ output. `BuildStats` is
//! only used for optional printing in C++ and is not ported.

use crate::error::{OrFormat, Result};
use crate::io::{Reader, Writer};
use crate::math::{AABox, Ray, Vector3, fuzzy_eq_f32, fuzzy_ne, std_max, std_min};

/// `MAX_STACK_SIZE` (BoundingIntervalHierarchy.h): maximum build depth.
pub const MAX_STACK_SIZE: i32 = 64;

/// Exceptions thrown by `BIH::subdivide` (`std::logic_error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BihBuildError {
    #[error("negative node extents")]
    NegativeNodeExtents,
    #[error("invalid node overlap")]
    InvalidNodeOverlap,
}

/// `AABound` (BoundingIntervalHierarchy.h).
#[derive(Debug, Clone, Copy)]
struct AABound {
    lo: Vector3,
    hi: Vector3,
}

/// `BIH::buildData`.
struct BuildData<'a> {
    indices: Vec<u32>,
    prim_bound: &'a [AABox],
    max_prims: i32,
}

/// `BIH` — flat node array (`tree`), leaf object indices (`objects`) and
/// the overall `bounds`.
#[derive(Debug, Clone, PartialEq)]
pub struct Bih {
    tree: Vec<u32>,
    objects: Vec<u32>,
    bounds: AABox,
}

impl Default for Bih {
    /// `BIH()` — `init_empty()` with default (NaN) bounds.
    fn default() -> Self {
        Self {
            tree: vec![3u32 << 30, 0, 0],
            objects: Vec::new(),
            bounds: AABox::EMPTY,
        }
    }
}

impl Bih {
    /// `BIH::init_empty` — keeps `bounds` untouched, like the C++.
    fn init_empty(&mut self) {
        self.tree.clear();
        self.objects.clear();
        self.tree.push(3u32 << 30);
        self.tree.extend([0, 0]);
    }

    /// `BIH::build(primitives, getBounds, leafSize)` with the per-primitive
    /// bounds already evaluated (`dat.primBound`).
    pub fn build(&mut self, prim_bounds: &[AABox], leaf_size: u32) -> Result<(), BihBuildError> {
        if prim_bounds.is_empty() {
            self.init_empty();
            return Ok(());
        }
        let num_prims = prim_bounds.len() as u32;
        let mut dat = BuildData {
            indices: (0..num_prims).collect(),
            prim_bound: prim_bounds,
            max_prims: leaf_size as i32,
        };
        self.bounds = prim_bounds[0];
        for b in prim_bounds {
            self.bounds.merge(b);
        }
        let mut temp_tree = Vec::new();
        self.build_hierarchy(&mut temp_tree, &mut dat)?;
        self.objects = dat.indices;
        self.tree = temp_tree;
        Ok(())
    }

    /// Convenience wrapper mirroring the templated C++ `build` signature.
    pub fn build_with<T>(
        &mut self,
        primitives: &[T],
        get_bounds: impl Fn(&T) -> AABox,
        leaf_size: u32,
    ) -> Result<(), BihBuildError> {
        let bounds: Vec<AABox> = primitives.iter().map(get_bounds).collect();
        self.build(&bounds, leaf_size)
    }

    /// `BIH::buildHierarchy`.
    fn build_hierarchy(
        &self,
        temp_tree: &mut Vec<u32>,
        dat: &mut BuildData<'_>,
    ) -> Result<(), BihBuildError> {
        temp_tree.push(3u32 << 30); // dummy leaf
        temp_tree.extend([0, 0]);
        let mut grid_box = AABound {
            lo: self.bounds.low(),
            hi: self.bounds.high(),
        };
        let mut node_box = grid_box;
        let right = dat.indices.len() as i32 - 1;
        subdivide(0, right, temp_tree, dat, &mut grid_box, &mut node_box, 0, 1)
    }

    /// `BIH::primCount`.
    pub fn prim_count(&self) -> u32 {
        self.objects.len() as u32
    }

    pub fn tree(&self) -> &[u32] {
        &self.tree
    }

    pub fn objects(&self) -> &[u32] {
        &self.objects
    }

    pub fn bounds(&self) -> &AABox {
        &self.bounds
    }

    /// `BIH::writeToFile`.
    pub fn write_to(&self, w: &mut impl Writer) {
        w.put_vector3(self.bounds.low());
        w.put_vector3(self.bounds.high());
        w.put_u32(self.tree.len() as u32);
        for &v in &self.tree {
            w.put_u32(v);
        }
        w.put_u32(self.objects.len() as u32);
        for &v in &self.objects {
            w.put_u32(v);
        }
    }

    /// `BIH::readFromFile`.
    pub fn read_from(r: &mut Reader<'_>) -> Result<Self> {
        let lo = r.vector3().or_format("BIH bounds")?;
        let hi = r.vector3().or_format("BIH bounds")?;
        let tree_size = r.u32().or_format("BIH tree size")?;
        let tree = r.u32_vec(tree_size as usize).or_format("BIH tree")?;
        let count = r.u32().or_format("BIH object count")?;
        let objects = r.u32_vec(count as usize).or_format("BIH objects")?;
        Ok(Self {
            tree,
            objects,
            bounds: AABox::new(lo, hi),
        })
    }

    /// `BIH::intersectRay`. The callback receives `(ray, object index,
    /// max distance, stop_at_first)` and returns whether it hit.
    pub fn intersect_ray<F>(
        &self,
        r: &Ray,
        callback: &mut F,
        max_dist: &mut f32,
        stop_at_first: bool,
    ) where
        F: FnMut(&Ray, u32, &mut f32, bool) -> bool,
    {
        let mut interval_min = -1.0f32;
        let mut interval_max = -1.0f32;
        let org = r.origin();
        let dir = r.direction();
        let inv_dir = r.inv_direction();
        for i in 0..3 {
            if fuzzy_ne(dir[i], 0.0) {
                let mut t1 = (self.bounds.low()[i] - org[i]) * inv_dir[i];
                let mut t2 = (self.bounds.high()[i] - org[i]) * inv_dir[i];
                if t1 > t2 {
                    std::mem::swap(&mut t1, &mut t2);
                }
                if t1 > interval_min {
                    interval_min = t1;
                }
                if t2 < interval_max || interval_max < 0.0 {
                    interval_max = t2;
                }
                if interval_max <= 0.0 || interval_min >= *max_dist {
                    return;
                }
            }
        }
        if interval_min > interval_max {
            return;
        }
        interval_min = std_max(interval_min, 0.0);
        interval_max = std_min(interval_max, *max_dist);

        let mut offset_front = [0usize; 3];
        let mut offset_back = [0usize; 3];
        let mut offset_front3 = [0usize; 3];
        let mut offset_back3 = [0usize; 3];
        for i in 0..3 {
            offset_front[i] = (dir[i].to_bits() >> 31) as usize;
            offset_back[i] = offset_front[i] ^ 1;
            offset_front3[i] = offset_front[i] * 3;
            offset_back3[i] = offset_back[i] * 3;
            offset_front[i] += 1;
            offset_back[i] += 1;
        }

        let mut stack: Vec<(usize, f32, f32)> = Vec::with_capacity(MAX_STACK_SIZE as usize);
        let mut node = 0usize;
        loop {
            loop {
                let tn = self.tree[node];
                let axis = ((tn & (3 << 30)) >> 30) as usize;
                let bvh2 = tn & (1 << 29) != 0;
                let mut offset = (tn & !(7 << 29)) as usize;
                if !bvh2 {
                    if axis < 3 {
                        // "normal" interior node
                        let tf = (f32::from_bits(self.tree[node + offset_front[axis]]) - org[axis])
                            * inv_dir[axis];
                        let tb = (f32::from_bits(self.tree[node + offset_back[axis]]) - org[axis])
                            * inv_dir[axis];
                        if tf < interval_min && tb > interval_max {
                            break;
                        }
                        let back = offset + offset_back3[axis];
                        node = back;
                        if tf < interval_min {
                            interval_min = if tb >= interval_min { tb } else { interval_min };
                            continue;
                        }
                        node = offset + offset_front3[axis];
                        if tb > interval_max {
                            interval_max = if tf <= interval_max { tf } else { interval_max };
                            continue;
                        }
                        stack.push((
                            back,
                            if tb >= interval_min { tb } else { interval_min },
                            interval_max,
                        ));
                        interval_max = if tf <= interval_max { tf } else { interval_max };
                        continue;
                    }
                    // leaf - test some objects
                    let mut n = self.tree[node + 1] as i32;
                    while n > 0 {
                        let hit = callback(r, self.objects[offset], max_dist, stop_at_first);
                        if stop_at_first && hit {
                            return;
                        }
                        n -= 1;
                        offset += 1;
                    }
                    break;
                }
                if axis > 2 {
                    return; // should not happen
                }
                let tf = (f32::from_bits(self.tree[node + offset_front[axis]]) - org[axis])
                    * inv_dir[axis];
                let tb = (f32::from_bits(self.tree[node + offset_back[axis]]) - org[axis])
                    * inv_dir[axis];
                node = offset;
                interval_min = if tf >= interval_min { tf } else { interval_min };
                interval_max = if tb <= interval_max { tb } else { interval_max };
                if interval_min > interval_max {
                    break;
                }
            }
            loop {
                let Some((n, tnear, tfar)) = stack.pop() else {
                    return;
                };
                interval_min = tnear;
                if *max_dist < interval_min {
                    continue;
                }
                node = n;
                interval_max = tfar;
                break;
            }
        }
    }

    /// `BIH::intersectPoint`. The callback receives `(point, object index)`.
    pub fn intersect_point<F>(&self, p: Vector3, callback: &mut F)
    where
        F: FnMut(&Vector3, u32),
    {
        if !self.bounds.contains(p) {
            return;
        }
        let mut stack: Vec<usize> = Vec::with_capacity(MAX_STACK_SIZE as usize);
        let mut node = 0usize;
        loop {
            loop {
                let tn = self.tree[node];
                let axis = ((tn & (3 << 30)) >> 30) as usize;
                let bvh2 = tn & (1 << 29) != 0;
                let mut offset = (tn & !(7 << 29)) as usize;
                if !bvh2 {
                    if axis < 3 {
                        let tl = f32::from_bits(self.tree[node + 1]);
                        let tr = f32::from_bits(self.tree[node + 2]);
                        if tl < p[axis] && tr > p[axis] {
                            break;
                        }
                        let right = offset + 3;
                        node = right;
                        if tl < p[axis] {
                            continue;
                        }
                        node = offset; // left
                        if tr > p[axis] {
                            continue;
                        }
                        stack.push(right);
                        continue;
                    }
                    let mut n = self.tree[node + 1] as i32;
                    while n > 0 {
                        callback(&p, self.objects[offset]);
                        n -= 1;
                        offset += 1;
                    }
                    break;
                }
                if axis > 2 {
                    return;
                }
                let tl = f32::from_bits(self.tree[node + 1]);
                let tr = f32::from_bits(self.tree[node + 2]);
                node = offset;
                if tl > p[axis] || tr < p[axis] {
                    break;
                }
            }
            let Some(n) = stack.pop() else {
                return;
            };
            node = n;
        }
    }
}

/// `BIH::createNode` — writes a leaf node.
fn create_node(temp_tree: &mut [u32], node_index: i32, left: i32, right: i32) {
    let ni = node_index as usize;
    temp_tree[ni] = (3u32 << 30) | left as u32;
    temp_tree[ni + 1] = (right - left + 1) as u32;
}

fn alloc_node(temp_tree: &mut Vec<u32>) {
    temp_tree.extend([0, 0, 0]);
}

/// `BIH::subdivide`.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn subdivide(
    left: i32,
    mut right: i32,
    temp_tree: &mut Vec<u32>,
    dat: &mut BuildData<'_>,
    grid_box: &mut AABound,
    node_box: &mut AABound,
    mut node_index: i32,
    mut depth: i32,
) -> Result<(), BihBuildError> {
    if (right - left + 1) <= dat.max_prims || depth >= MAX_STACK_SIZE {
        create_node(temp_tree, node_index, left, right);
        return Ok(());
    }
    // calculate extents
    let mut axis: i32 = -1;
    let mut prev_axis: i32;
    let mut right_orig: i32;
    let mut clip_l: f32;
    let mut clip_r: f32;
    let mut prev_clip = f32::NAN;
    let mut split = f32::NAN;
    let mut prev_split: f32;
    let mut was_left = true;
    loop {
        prev_axis = axis;
        prev_split = split;
        // perform quick consistency checks
        let d = grid_box.hi - grid_box.lo;
        if d.x < 0.0 || d.y < 0.0 || d.z < 0.0 {
            return Err(BihBuildError::NegativeNodeExtents);
        }
        for i in 0..3 {
            if node_box.hi[i] < grid_box.lo[i] || node_box.lo[i] > grid_box.hi[i] {
                return Err(BihBuildError::InvalidNodeOverlap);
            }
        }
        // find longest axis
        let ax = d.primary_axis();
        axis = ax as i32;
        split = 0.5f32 * (grid_box.lo[ax] + grid_box.hi[ax]);
        // partition L/R subsets
        clip_l = -f32::INFINITY;
        clip_r = f32::INFINITY;
        right_orig = right; // save this for later
        let mut node_l = f32::INFINITY;
        let mut node_r = -f32::INFINITY;
        let mut i = left;
        while i <= right {
            let obj = dat.indices[i as usize] as usize;
            let minb = dat.prim_bound[obj].low()[ax];
            let maxb = dat.prim_bound[obj].high()[ax];
            let center = (minb + maxb) * 0.5f32;
            if center <= split {
                // stay left
                i += 1;
                if clip_l < maxb {
                    clip_l = maxb;
                }
            } else {
                // move to the right most
                dat.indices.swap(i as usize, right as usize);
                right -= 1;
                if clip_r > minb {
                    clip_r = minb;
                }
            }
            node_l = std_min(node_l, minb);
            node_r = std_max(node_r, maxb);
        }
        // check for empty space
        if node_l > node_box.lo[ax] && node_r < node_box.hi[ax] {
            let node_box_w = node_box.hi[ax] - node_box.lo[ax];
            let node_new_w = node_r - node_l;
            // node box is too big compare to space occupied by primitives?
            if 1.3f32 * node_new_w < node_box_w {
                let next_index = temp_tree.len() as i32;
                alloc_node(temp_tree);
                let ni = node_index as usize;
                temp_tree[ni] = ((axis as u32) << 30) | (1 << 29) | next_index as u32;
                temp_tree[ni + 1] = node_l.to_bits();
                temp_tree[ni + 2] = node_r.to_bits();
                node_box.lo[ax] = node_l;
                node_box.hi[ax] = node_r;
                return subdivide(
                    left,
                    right_orig,
                    temp_tree,
                    dat,
                    grid_box,
                    node_box,
                    next_index,
                    depth + 1,
                );
            }
        }
        // ensure we are making progress in the subdivision
        if right == right_orig {
            // all left
            if prev_axis == axis && fuzzy_eq_f32(prev_split, split) {
                // we are stuck here - create a leaf
                create_node(temp_tree, node_index, left, right);
                return Ok(());
            }
            if clip_l <= split {
                // keep looping on left half
                grid_box.hi[ax] = split;
                prev_clip = clip_l;
                was_left = true;
                continue;
            }
            grid_box.hi[ax] = split;
            prev_clip = f32::NAN;
        } else if left > right {
            // all right
            right = right_orig;
            if prev_axis == axis && fuzzy_eq_f32(prev_split, split) {
                // we are stuck here - create a leaf
                create_node(temp_tree, node_index, left, right);
                return Ok(());
            }
            if clip_r >= split {
                // keep looping on right half
                grid_box.lo[ax] = split;
                prev_clip = clip_r;
                was_left = false;
                continue;
            }
            grid_box.lo[ax] = split;
            prev_clip = f32::NAN;
        } else {
            // we are actually splitting stuff
            if prev_axis != -1 && !prev_clip.is_nan() {
                // second time through - lets create the previous split
                // since it produced empty space
                let next_index = temp_tree.len() as i32;
                alloc_node(temp_tree);
                let ni = node_index as usize;
                if was_left {
                    // create a node with a left child
                    temp_tree[ni] = ((prev_axis as u32) << 30) | next_index as u32;
                    temp_tree[ni + 1] = prev_clip.to_bits();
                    temp_tree[ni + 2] = f32::INFINITY.to_bits();
                } else {
                    // create a node with a right child
                    temp_tree[ni] = ((prev_axis as u32) << 30) | (next_index - 3) as u32;
                    temp_tree[ni + 1] = (-f32::INFINITY).to_bits();
                    temp_tree[ni + 2] = prev_clip.to_bits();
                }
                depth += 1;
                node_index = next_index;
            }
            break;
        }
    }
    let ax = axis as usize;
    // compute index of child nodes
    let mut next_index = temp_tree.len() as i32;
    // allocate left node
    let nl = right - left + 1;
    let nr = right_orig - (right + 1) + 1;
    if nl > 0 {
        alloc_node(temp_tree);
    } else {
        next_index -= 3;
    }
    // allocate right node
    if nr > 0 {
        alloc_node(temp_tree);
    }
    let ni = node_index as usize;
    temp_tree[ni] = ((axis as u32) << 30) | next_index as u32;
    temp_tree[ni + 1] = clip_l.to_bits();
    temp_tree[ni + 2] = clip_r.to_bits();
    // prepare L/R child boxes
    let mut grid_box_l = *grid_box;
    let mut grid_box_r = *grid_box;
    let mut node_box_l = *node_box;
    let mut node_box_r = *node_box;
    grid_box_l.hi[ax] = split;
    grid_box_r.lo[ax] = split;
    node_box_l.hi[ax] = clip_l;
    node_box_r.lo[ax] = clip_r;
    // recurse
    if nl > 0 {
        subdivide(
            left,
            right,
            temp_tree,
            dat,
            &mut grid_box_l,
            &mut node_box_l,
            next_index,
            depth + 1,
        )?;
    }
    if nr > 0 {
        subdivide(
            right + 1,
            right_orig,
            temp_tree,
            dat,
            &mut grid_box_r,
            &mut node_box_r,
            next_index + 3,
            depth + 1,
        )?;
    }
    Ok(())
}
