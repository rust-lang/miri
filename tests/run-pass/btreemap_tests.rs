#![allow(dead_code)]
#![feature(btree_drain_filter)]
#![feature(map_first_last)]
#![feature(map_into_keys_values)]
use std::boxed::Box;
use std::collections::btree_map::Entry::{Occupied, Vacant};
use std::collections::BTreeMap;
use std::convert::TryFrom;
use std::fmt::Debug;
use std::iter::{self, FromIterator};
use std::mem;
use std::ops::Bound::{self, Excluded, Included, Unbounded};
use std::ops::RangeBounds;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::string::{String, ToString};
use std::sync::atomic::{AtomicUsize, Ordering};

struct DeterministicRng {
    x: u32,
    y: u32,
    z: u32,
    w: u32,
}

impl DeterministicRng {
    fn new() -> Self {
        DeterministicRng { x: 0x193a6754, y: 0xa8a7d469, z: 0x97830e05, w: 0x113ba7bb }
    }

    fn next(&mut self) -> u32 {
        let x = self.x;
        let t = x ^ (x << 11);
        self.x = self.y;
        self.y = self.z;
        self.z = self.w;
        let w_ = self.w;
        self.w = w_ ^ (w_ >> 19) ^ (t ^ (t >> 8));
        self.w
    }
}

// Capacity of a tree with a single level,
// i.e., a tree who's root is a leaf node at height 0.
const NODE_CAPACITY: usize = 11;

// Minimum number of elements to insert, to guarantee a tree with 2 levels,
// i.e., a tree who's root is an internal node at height 1, with edges to leaf nodes.
// It's not the minimum size: removing an element from such a tree does not always reduce height.
const MIN_INSERTS_HEIGHT_1: usize = NODE_CAPACITY + 1;

// Minimum number of elements to insert in ascending order, to guarantee a tree with 3 levels,
// i.e., a tree who's root is an internal node at height 2, with edges to more internal nodes.
// It's not the minimum size: removing an element from such a tree does not always reduce height.
const MIN_INSERTS_HEIGHT_2: usize = 89;

// Gather all references from a mutable iterator and make sure Miri notices if
// using them is dangerous.
fn test_all_refs<'a, T: 'a>(dummy: &mut T, iter: impl Iterator<Item = &'a mut T>) {
    // Gather all those references.
    let mut refs: Vec<&mut T> = iter.collect();
    // Use them all. Twice, to be sure we got all interleavings.
    for r in refs.iter_mut() {
        mem::swap(dummy, r);
    }
    for r in refs {
        mem::swap(dummy, r);
    }
}

fn test_basic_large() {
    let mut map = BTreeMap::new();
    // Miri is too slow
    let size = if cfg!(miri) { MIN_INSERTS_HEIGHT_2 } else { 10000 };
    let size = size + (size % 2); // round up to even number
    assert_eq!(map.len(), 0);

    for i in 0..size {
        assert_eq!(map.insert(i, 10 * i), None);
        assert_eq!(map.len(), i + 1);
    }

    assert_eq!(map.first_key_value(), Some((&0, &0)));
    assert_eq!(map.last_key_value(), Some((&(size - 1), &(10 * (size - 1)))));
    assert_eq!(map.first_entry().unwrap().key(), &0);
    assert_eq!(map.last_entry().unwrap().key(), &(size - 1));

    for i in 0..size {
        assert_eq!(map.get(&i).unwrap(), &(i * 10));
    }

    for i in size..size * 2 {
        assert_eq!(map.get(&i), None);
    }

    for i in 0..size {
        assert_eq!(map.insert(i, 100 * i), Some(10 * i));
        assert_eq!(map.len(), size);
    }

    for i in 0..size {
        assert_eq!(map.get(&i).unwrap(), &(i * 100));
    }

    for i in 0..size / 2 {
        assert_eq!(map.remove(&(i * 2)), Some(i * 200));
        assert_eq!(map.len(), size - i - 1);
    }

    for i in 0..size / 2 {
        assert_eq!(map.get(&(2 * i)), None);
        assert_eq!(map.get(&(2 * i + 1)).unwrap(), &(i * 200 + 100));
    }

    for i in 0..size / 2 {
        assert_eq!(map.remove(&(2 * i)), None);
        assert_eq!(map.remove(&(2 * i + 1)), Some(i * 200 + 100));
        assert_eq!(map.len(), size / 2 - i - 1);
    }
}

fn test_basic_small() {
    let mut map = BTreeMap::new();
    // Empty, root is absent (None):
    assert_eq!(map.remove(&1), None);
    assert_eq!(map.len(), 0);
    assert_eq!(map.get(&1), None);
    assert_eq!(map.get_mut(&1), None);
    assert_eq!(map.first_key_value(), None);
    assert_eq!(map.last_key_value(), None);
    assert_eq!(map.keys().count(), 0);
    assert_eq!(map.values().count(), 0);
    assert_eq!(map.range(..).next(), None);
    assert_eq!(map.range(..1).next(), None);
    assert_eq!(map.range(1..).next(), None);
    assert_eq!(map.range(1..=1).next(), None);
    assert_eq!(map.range(1..2).next(), None);
    assert_eq!(map.insert(1, 1), None);

    // 1 key-value pair:
    assert_eq!(map.len(), 1);
    assert_eq!(map.get(&1), Some(&1));
    assert_eq!(map.get_mut(&1), Some(&mut 1));
    assert_eq!(map.first_key_value(), Some((&1, &1)));
    assert_eq!(map.last_key_value(), Some((&1, &1)));
    assert_eq!(map.keys().collect::<Vec<_>>(), vec![&1]);
    assert_eq!(map.values().collect::<Vec<_>>(), vec![&1]);
    assert_eq!(map.insert(1, 2), Some(1));
    assert_eq!(map.len(), 1);
    assert_eq!(map.get(&1), Some(&2));
    assert_eq!(map.get_mut(&1), Some(&mut 2));
    assert_eq!(map.first_key_value(), Some((&1, &2)));
    assert_eq!(map.last_key_value(), Some((&1, &2)));
    assert_eq!(map.keys().collect::<Vec<_>>(), vec![&1]);
    assert_eq!(map.values().collect::<Vec<_>>(), vec![&2]);
    assert_eq!(map.insert(2, 4), None);

    // 2 key-value pairs:
    assert_eq!(map.len(), 2);
    assert_eq!(map.get(&2), Some(&4));
    assert_eq!(map.get_mut(&2), Some(&mut 4));
    assert_eq!(map.first_key_value(), Some((&1, &2)));
    assert_eq!(map.last_key_value(), Some((&2, &4)));
    assert_eq!(map.keys().collect::<Vec<_>>(), vec![&1, &2]);
    assert_eq!(map.values().collect::<Vec<_>>(), vec![&2, &4]);
    assert_eq!(map.remove(&1), Some(2));

    // 1 key-value pair:
    assert_eq!(map.len(), 1);
    assert_eq!(map.get(&1), None);
    assert_eq!(map.get_mut(&1), None);
    assert_eq!(map.get(&2), Some(&4));
    assert_eq!(map.get_mut(&2), Some(&mut 4));
    assert_eq!(map.first_key_value(), Some((&2, &4)));
    assert_eq!(map.last_key_value(), Some((&2, &4)));
    assert_eq!(map.keys().collect::<Vec<_>>(), vec![&2]);
    assert_eq!(map.values().collect::<Vec<_>>(), vec![&4]);
    assert_eq!(map.remove(&2), Some(4));

    // Empty but root is owned (Some(...)):
    assert_eq!(map.len(), 0);
    assert_eq!(map.get(&1), None);
    assert_eq!(map.get_mut(&1), None);
    assert_eq!(map.first_key_value(), None);
    assert_eq!(map.last_key_value(), None);
    assert_eq!(map.keys().count(), 0);
    assert_eq!(map.values().count(), 0);
    assert_eq!(map.range(..).next(), None);
    assert_eq!(map.range(..1).next(), None);
    assert_eq!(map.range(1..).next(), None);
    assert_eq!(map.range(1..=1).next(), None);
    assert_eq!(map.range(1..2).next(), None);
    assert_eq!(map.remove(&1), None);
}

fn test_iter() {
    // Miri is too slow
    let size = if cfg!(miri) { 200 } else { 10000 };

    let mut map: BTreeMap<_, _> = (0..size).map(|i| (i, i)).collect();

    fn test<T>(size: usize, mut iter: T)
    where
        T: Iterator<Item = (usize, usize)>,
    {
        for i in 0..size {
            assert_eq!(iter.size_hint(), (size - i, Some(size - i)));
            assert_eq!(iter.next().unwrap(), (i, i));
        }
        assert_eq!(iter.size_hint(), (0, Some(0)));
        assert_eq!(iter.next(), None);
    }
    test(size, map.iter().map(|(&k, &v)| (k, v)));
    test(size, map.iter_mut().map(|(&k, &mut v)| (k, v)));
    test(size, map.into_iter());
}

fn test_iter_rev() {
    // Miri is too slow
    let size = if cfg!(miri) { 200 } else { 10000 };

    let mut map: BTreeMap<_, _> = (0..size).map(|i| (i, i)).collect();

    fn test<T>(size: usize, mut iter: T)
    where
        T: Iterator<Item = (usize, usize)>,
    {
        for i in 0..size {
            assert_eq!(iter.size_hint(), (size - i, Some(size - i)));
            assert_eq!(iter.next().unwrap(), (size - i - 1, size - i - 1));
        }
        assert_eq!(iter.size_hint(), (0, Some(0)));
        assert_eq!(iter.next(), None);
    }
    test(size, map.iter().rev().map(|(&k, &v)| (k, v)));
    test(size, map.iter_mut().rev().map(|(&k, &mut v)| (k, v)));
    test(size, map.into_iter().rev());
}

/// Specifically tests iter_mut's ability to mutate the value of pairs in-line
fn do_test_iter_mut_mutation<T>(size: usize)
where
    T: Copy + Debug + Ord + TryFrom<usize>,
    <T as TryFrom<usize>>::Error: Debug,
{
    let zero = T::try_from(0).unwrap();
    let mut map: BTreeMap<T, T> = (0..size).map(|i| (T::try_from(i).unwrap(), zero)).collect();

    // Forward and backward iteration sees enough pairs (also tested elsewhere)
    assert_eq!(map.iter_mut().count(), size);
    assert_eq!(map.iter_mut().rev().count(), size);

    // Iterate forwards, trying to mutate to unique values
    for (i, (k, v)) in map.iter_mut().enumerate() {
        assert_eq!(*k, T::try_from(i).unwrap());
        assert_eq!(*v, zero);
        *v = T::try_from(i + 1).unwrap();
    }

    // Iterate backwards, checking that mutations succeeded and trying to mutate again
    for (i, (k, v)) in map.iter_mut().rev().enumerate() {
        assert_eq!(*k, T::try_from(size - i - 1).unwrap());
        assert_eq!(*v, T::try_from(size - i).unwrap());
        *v = T::try_from(2 * size - i).unwrap();
    }

    // Check that backward mutations succeeded
    for (i, (k, v)) in map.iter_mut().enumerate() {
        assert_eq!(*k, T::try_from(i).unwrap());
        assert_eq!(*v, T::try_from(size + i + 1).unwrap());
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(align(32))]
struct Align32(usize);

impl TryFrom<usize> for Align32 {
    type Error = ();

    fn try_from(s: usize) -> Result<Align32, ()> {
        Ok(Align32(s))
    }
}

fn test_iter_mut_mutation() {
    // Check many alignments and trees with roots at various heights.
    do_test_iter_mut_mutation::<u8>(0);
    do_test_iter_mut_mutation::<u8>(1);
    do_test_iter_mut_mutation::<u8>(MIN_INSERTS_HEIGHT_1);
    do_test_iter_mut_mutation::<u8>(MIN_INSERTS_HEIGHT_2);
    do_test_iter_mut_mutation::<u16>(1);
    do_test_iter_mut_mutation::<u16>(MIN_INSERTS_HEIGHT_1);
    do_test_iter_mut_mutation::<u16>(MIN_INSERTS_HEIGHT_2);
    do_test_iter_mut_mutation::<u32>(1);
    do_test_iter_mut_mutation::<u32>(MIN_INSERTS_HEIGHT_1);
    do_test_iter_mut_mutation::<u32>(MIN_INSERTS_HEIGHT_2);
    do_test_iter_mut_mutation::<u64>(1);
    do_test_iter_mut_mutation::<u64>(MIN_INSERTS_HEIGHT_1);
    do_test_iter_mut_mutation::<u64>(MIN_INSERTS_HEIGHT_2);
    do_test_iter_mut_mutation::<u128>(1);
    do_test_iter_mut_mutation::<u128>(MIN_INSERTS_HEIGHT_1);
    do_test_iter_mut_mutation::<u128>(MIN_INSERTS_HEIGHT_2);
    do_test_iter_mut_mutation::<Align32>(1);
    do_test_iter_mut_mutation::<Align32>(MIN_INSERTS_HEIGHT_1);
    do_test_iter_mut_mutation::<Align32>(MIN_INSERTS_HEIGHT_2);
}

fn test_values_mut() {
    let mut a: BTreeMap<_, _> = (0..MIN_INSERTS_HEIGHT_2).map(|i| (i, i)).collect();
    test_all_refs(&mut 13, a.values_mut());
}

fn test_values_mut_mutation() {
    let mut a = BTreeMap::new();
    a.insert(1, String::from("hello"));
    a.insert(2, String::from("goodbye"));

    for value in a.values_mut() {
        value.push_str("!");
    }

    let values: Vec<String> = a.values().cloned().collect();
    assert_eq!(values, [String::from("hello!"), String::from("goodbye!")]);
}

fn test_iter_entering_root_twice() {
    let mut map: BTreeMap<_, _> = (0..2).map(|i| (i, i)).collect();
    let mut it = map.iter_mut();
    let front = it.next().unwrap();
    let back = it.next_back().unwrap();
    assert_eq!(front, (&0, &mut 0));
    assert_eq!(back, (&1, &mut 1));
    *front.1 = 24;
    *back.1 = 42;
    assert_eq!(front, (&0, &mut 24));
    assert_eq!(back, (&1, &mut 42));
}

fn test_iter_descending_to_same_node_twice() {
    let mut map: BTreeMap<_, _> = (0..MIN_INSERTS_HEIGHT_1).map(|i| (i, i)).collect();
    let mut it = map.iter_mut();
    // Descend into first child.
    let front = it.next().unwrap();
    // Descend into first child again, after running through second child.
    while it.next_back().is_some() {}
    // Check immutable access.
    assert_eq!(front, (&0, &mut 0));
    // Perform mutable access.
    *front.1 = 42;
}

fn test_iter_mixed() {
    // Miri is too slow
    let size = if cfg!(miri) { 200 } else { 10000 };

    let mut map: BTreeMap<_, _> = (0..size).map(|i| (i, i)).collect();

    fn test<T>(size: usize, mut iter: T)
    where
        T: Iterator<Item = (usize, usize)> + DoubleEndedIterator,
    {
        for i in 0..size / 4 {
            assert_eq!(iter.size_hint(), (size - i * 2, Some(size - i * 2)));
            assert_eq!(iter.next().unwrap(), (i, i));
            assert_eq!(iter.next_back().unwrap(), (size - i - 1, size - i - 1));
        }
        for i in size / 4..size * 3 / 4 {
            assert_eq!(iter.size_hint(), (size * 3 / 4 - i, Some(size * 3 / 4 - i)));
            assert_eq!(iter.next().unwrap(), (i, i));
        }
        assert_eq!(iter.size_hint(), (0, Some(0)));
        assert_eq!(iter.next(), None);
    }
    test(size, map.iter().map(|(&k, &v)| (k, v)));
    test(size, map.iter_mut().map(|(&k, &mut v)| (k, v)));
    test(size, map.into_iter());
}

fn test_iter_min_max() {
    let mut a = BTreeMap::new();
    assert_eq!(a.iter().min(), None);
    assert_eq!(a.iter().max(), None);
    assert_eq!(a.iter_mut().min(), None);
    assert_eq!(a.iter_mut().max(), None);
    assert_eq!(a.range(..).min(), None);
    assert_eq!(a.range(..).max(), None);
    assert_eq!(a.range_mut(..).min(), None);
    assert_eq!(a.range_mut(..).max(), None);
    assert_eq!(a.keys().min(), None);
    assert_eq!(a.keys().max(), None);
    assert_eq!(a.values().min(), None);
    assert_eq!(a.values().max(), None);
    assert_eq!(a.values_mut().min(), None);
    assert_eq!(a.values_mut().max(), None);
    a.insert(1, 42);
    a.insert(2, 24);
    assert_eq!(a.iter().min(), Some((&1, &42)));
    assert_eq!(a.iter().max(), Some((&2, &24)));
    assert_eq!(a.iter_mut().min(), Some((&1, &mut 42)));
    assert_eq!(a.iter_mut().max(), Some((&2, &mut 24)));
    assert_eq!(a.range(..).min(), Some((&1, &42)));
    assert_eq!(a.range(..).max(), Some((&2, &24)));
    assert_eq!(a.range_mut(..).min(), Some((&1, &mut 42)));
    assert_eq!(a.range_mut(..).max(), Some((&2, &mut 24)));
    assert_eq!(a.keys().min(), Some(&1));
    assert_eq!(a.keys().max(), Some(&2));
    assert_eq!(a.values().min(), Some(&24));
    assert_eq!(a.values().max(), Some(&42));
    assert_eq!(a.values_mut().min(), Some(&mut 24));
    assert_eq!(a.values_mut().max(), Some(&mut 42));
}

fn range_keys(map: &BTreeMap<i32, i32>, range: impl RangeBounds<i32>) -> Vec<i32> {
    map.range(range)
        .map(|(&k, &v)| {
            assert_eq!(k, v);
            k
        })
        .collect()
}

fn test_range_small() {
    let size = 4;

    let map: BTreeMap<_, _> = (1..=size).map(|i| (i, i)).collect();
    let all: Vec<_> = (1..=size).collect();
    let (first, last) = (vec![all[0]], vec![all[size as usize - 1]]);

    assert_eq!(range_keys(&map, (Excluded(0), Excluded(size + 1))), all);
    assert_eq!(range_keys(&map, (Excluded(0), Included(size + 1))), all);
    assert_eq!(range_keys(&map, (Excluded(0), Included(size))), all);
    assert_eq!(range_keys(&map, (Excluded(0), Unbounded)), all);
    assert_eq!(range_keys(&map, (Included(0), Excluded(size + 1))), all);
    assert_eq!(range_keys(&map, (Included(0), Included(size + 1))), all);
    assert_eq!(range_keys(&map, (Included(0), Included(size))), all);
    assert_eq!(range_keys(&map, (Included(0), Unbounded)), all);
    assert_eq!(range_keys(&map, (Included(1), Excluded(size + 1))), all);
    assert_eq!(range_keys(&map, (Included(1), Included(size + 1))), all);
    assert_eq!(range_keys(&map, (Included(1), Included(size))), all);
    assert_eq!(range_keys(&map, (Included(1), Unbounded)), all);
    assert_eq!(range_keys(&map, (Unbounded, Excluded(size + 1))), all);
    assert_eq!(range_keys(&map, (Unbounded, Included(size + 1))), all);
    assert_eq!(range_keys(&map, (Unbounded, Included(size))), all);
    assert_eq!(range_keys(&map, ..), all);

    assert_eq!(range_keys(&map, (Excluded(0), Excluded(1))), vec![]);
    assert_eq!(range_keys(&map, (Excluded(0), Included(0))), vec![]);
    assert_eq!(range_keys(&map, (Included(0), Included(0))), vec![]);
    assert_eq!(range_keys(&map, (Included(0), Excluded(1))), vec![]);
    assert_eq!(range_keys(&map, (Unbounded, Excluded(1))), vec![]);
    assert_eq!(range_keys(&map, (Unbounded, Included(0))), vec![]);
    assert_eq!(range_keys(&map, (Excluded(0), Excluded(2))), first);
    assert_eq!(range_keys(&map, (Excluded(0), Included(1))), first);
    assert_eq!(range_keys(&map, (Included(0), Excluded(2))), first);
    assert_eq!(range_keys(&map, (Included(0), Included(1))), first);
    assert_eq!(range_keys(&map, (Included(1), Excluded(2))), first);
    assert_eq!(range_keys(&map, (Included(1), Included(1))), first);
    assert_eq!(range_keys(&map, (Unbounded, Excluded(2))), first);
    assert_eq!(range_keys(&map, (Unbounded, Included(1))), first);
    assert_eq!(range_keys(&map, (Excluded(size - 1), Excluded(size + 1))), last);
    assert_eq!(range_keys(&map, (Excluded(size - 1), Included(size + 1))), last);
    assert_eq!(range_keys(&map, (Excluded(size - 1), Included(size))), last);
    assert_eq!(range_keys(&map, (Excluded(size - 1), Unbounded)), last);
    assert_eq!(range_keys(&map, (Included(size), Excluded(size + 1))), last);
    assert_eq!(range_keys(&map, (Included(size), Included(size + 1))), last);
    assert_eq!(range_keys(&map, (Included(size), Included(size))), last);
    assert_eq!(range_keys(&map, (Included(size), Unbounded)), last);
    assert_eq!(range_keys(&map, (Excluded(size), Excluded(size + 1))), vec![]);
    assert_eq!(range_keys(&map, (Excluded(size), Included(size))), vec![]);
    assert_eq!(range_keys(&map, (Excluded(size), Unbounded)), vec![]);
    assert_eq!(range_keys(&map, (Included(size + 1), Excluded(size + 1))), vec![]);
    assert_eq!(range_keys(&map, (Included(size + 1), Included(size + 1))), vec![]);
    assert_eq!(range_keys(&map, (Included(size + 1), Unbounded)), vec![]);

    assert_eq!(range_keys(&map, ..3), vec![1, 2]);
    assert_eq!(range_keys(&map, 3..), vec![3, 4]);
    assert_eq!(range_keys(&map, 2..=3), vec![2, 3]);
}

fn test_range_height_1() {
    // Tests tree with a root and 2 leaves. Depending on details we don't want or need
    // to rely upon, the single key at the root will be 6 or 7.

    let map: BTreeMap<_, _> = (1..=MIN_INSERTS_HEIGHT_1 as i32).map(|i| (i, i)).collect();
    for &root in &[6, 7] {
        assert_eq!(range_keys(&map, (Excluded(root), Excluded(root + 1))), vec![]);
        assert_eq!(range_keys(&map, (Excluded(root), Included(root + 1))), vec![root + 1]);
        assert_eq!(range_keys(&map, (Included(root), Excluded(root + 1))), vec![root]);
        assert_eq!(range_keys(&map, (Included(root), Included(root + 1))), vec![root, root + 1]);

        assert_eq!(range_keys(&map, (Excluded(root - 1), Excluded(root))), vec![]);
        assert_eq!(range_keys(&map, (Included(root - 1), Excluded(root))), vec![root - 1]);
        assert_eq!(range_keys(&map, (Excluded(root - 1), Included(root))), vec![root]);
        assert_eq!(range_keys(&map, (Included(root - 1), Included(root))), vec![root - 1, root]);
    }
}

fn test_range_large() {
    let size = 200;

    let map: BTreeMap<_, _> = (1..=size).map(|i| (i, i)).collect();
    let all: Vec<_> = (1..=size).collect();
    let (first, last) = (vec![all[0]], vec![all[size as usize - 1]]);

    assert_eq!(range_keys(&map, (Excluded(0), Excluded(size + 1))), all);
    assert_eq!(range_keys(&map, (Excluded(0), Included(size + 1))), all);
    assert_eq!(range_keys(&map, (Excluded(0), Included(size))), all);
    assert_eq!(range_keys(&map, (Excluded(0), Unbounded)), all);
    assert_eq!(range_keys(&map, (Included(0), Excluded(size + 1))), all);
    assert_eq!(range_keys(&map, (Included(0), Included(size + 1))), all);
    assert_eq!(range_keys(&map, (Included(0), Included(size))), all);
    assert_eq!(range_keys(&map, (Included(0), Unbounded)), all);
    assert_eq!(range_keys(&map, (Included(1), Excluded(size + 1))), all);
    assert_eq!(range_keys(&map, (Included(1), Included(size + 1))), all);
    assert_eq!(range_keys(&map, (Included(1), Included(size))), all);
    assert_eq!(range_keys(&map, (Included(1), Unbounded)), all);
    assert_eq!(range_keys(&map, (Unbounded, Excluded(size + 1))), all);
    assert_eq!(range_keys(&map, (Unbounded, Included(size + 1))), all);
    assert_eq!(range_keys(&map, (Unbounded, Included(size))), all);
    assert_eq!(range_keys(&map, ..), all);

    assert_eq!(range_keys(&map, (Excluded(0), Excluded(1))), vec![]);
    assert_eq!(range_keys(&map, (Excluded(0), Included(0))), vec![]);
    assert_eq!(range_keys(&map, (Included(0), Included(0))), vec![]);
    assert_eq!(range_keys(&map, (Included(0), Excluded(1))), vec![]);
    assert_eq!(range_keys(&map, (Unbounded, Excluded(1))), vec![]);
    assert_eq!(range_keys(&map, (Unbounded, Included(0))), vec![]);
    assert_eq!(range_keys(&map, (Excluded(0), Excluded(2))), first);
    assert_eq!(range_keys(&map, (Excluded(0), Included(1))), first);
    assert_eq!(range_keys(&map, (Included(0), Excluded(2))), first);
    assert_eq!(range_keys(&map, (Included(0), Included(1))), first);
    assert_eq!(range_keys(&map, (Included(1), Excluded(2))), first);
    assert_eq!(range_keys(&map, (Included(1), Included(1))), first);
    assert_eq!(range_keys(&map, (Unbounded, Excluded(2))), first);
    assert_eq!(range_keys(&map, (Unbounded, Included(1))), first);
    assert_eq!(range_keys(&map, (Excluded(size - 1), Excluded(size + 1))), last);
    assert_eq!(range_keys(&map, (Excluded(size - 1), Included(size + 1))), last);
    assert_eq!(range_keys(&map, (Excluded(size - 1), Included(size))), last);
    assert_eq!(range_keys(&map, (Excluded(size - 1), Unbounded)), last);
    assert_eq!(range_keys(&map, (Included(size), Excluded(size + 1))), last);
    assert_eq!(range_keys(&map, (Included(size), Included(size + 1))), last);
    assert_eq!(range_keys(&map, (Included(size), Included(size))), last);
    assert_eq!(range_keys(&map, (Included(size), Unbounded)), last);
    assert_eq!(range_keys(&map, (Excluded(size), Excluded(size + 1))), vec![]);
    assert_eq!(range_keys(&map, (Excluded(size), Included(size))), vec![]);
    assert_eq!(range_keys(&map, (Excluded(size), Unbounded)), vec![]);
    assert_eq!(range_keys(&map, (Included(size + 1), Excluded(size + 1))), vec![]);
    assert_eq!(range_keys(&map, (Included(size + 1), Included(size + 1))), vec![]);
    assert_eq!(range_keys(&map, (Included(size + 1), Unbounded)), vec![]);

    fn check<'a, L, R>(lhs: L, rhs: R)
    where
        L: IntoIterator<Item = (&'a i32, &'a i32)>,
        R: IntoIterator<Item = (&'a i32, &'a i32)>,
    {
        let lhs: Vec<_> = lhs.into_iter().collect();
        let rhs: Vec<_> = rhs.into_iter().collect();
        assert_eq!(lhs, rhs);
    }

    check(map.range(..=100), map.range(..101));
    check(map.range(5..=8), vec![(&5, &5), (&6, &6), (&7, &7), (&8, &8)]);
    check(map.range(-1..=2), vec![(&1, &1), (&2, &2)]);
}

fn test_range_inclusive_max_value() {
    let max = usize::MAX;
    let map: BTreeMap<_, _> = vec![(max, 0)].into_iter().collect();

    assert_eq!(map.range(max..=max).collect::<Vec<_>>(), &[(&max, &0)]);
}

fn test_range_equal_empty_cases() {
    let map: BTreeMap<_, _> = (0..5).map(|i| (i, i)).collect();
    assert_eq!(map.range((Included(2), Excluded(2))).next(), None);
    assert_eq!(map.range((Excluded(2), Included(2))).next(), None);
}

fn test_range_equal_excluded() {
    let map: BTreeMap<_, _> = (0..5).map(|i| (i, i)).collect();
    map.range((Excluded(2), Excluded(2)));
}

fn test_range_backwards_1() {
    let map: BTreeMap<_, _> = (0..5).map(|i| (i, i)).collect();
    map.range((Included(3), Included(2)));
}

fn test_range_backwards_2() {
    let map: BTreeMap<_, _> = (0..5).map(|i| (i, i)).collect();
    map.range((Included(3), Excluded(2)));
}

fn test_range_backwards_3() {
    let map: BTreeMap<_, _> = (0..5).map(|i| (i, i)).collect();
    map.range((Excluded(3), Included(2)));
}

fn test_range_backwards_4() {
    let map: BTreeMap<_, _> = (0..5).map(|i| (i, i)).collect();
    map.range((Excluded(3), Excluded(2)));
}

fn test_range_1000() {
    // Miri is too slow
    let size = if cfg!(miri) { MIN_INSERTS_HEIGHT_2 as u32 } else { 1000 };
    let map: BTreeMap<_, _> = (0..size).map(|i| (i, i)).collect();

    fn test(map: &BTreeMap<u32, u32>, size: u32, min: Bound<&u32>, max: Bound<&u32>) {
        let mut kvs = map.range((min, max)).map(|(&k, &v)| (k, v));
        let mut pairs = (0..size).map(|i| (i, i));

        for (kv, pair) in kvs.by_ref().zip(pairs.by_ref()) {
            assert_eq!(kv, pair);
        }
        assert_eq!(kvs.next(), None);
        assert_eq!(pairs.next(), None);
    }
    test(&map, size, Included(&0), Excluded(&size));
    test(&map, size, Unbounded, Excluded(&size));
    test(&map, size, Included(&0), Included(&(size - 1)));
    test(&map, size, Unbounded, Included(&(size - 1)));
    test(&map, size, Included(&0), Unbounded);
    test(&map, size, Unbounded, Unbounded);
}

fn test_range_borrowed_key() {
    let mut map = BTreeMap::new();
    map.insert("aardvark".to_string(), 1);
    map.insert("baboon".to_string(), 2);
    map.insert("coyote".to_string(), 3);
    map.insert("dingo".to_string(), 4);
    // NOTE: would like to use simply "b".."d" here...
    let mut iter = map.range::<str, _>((Included("b"), Excluded("d")));
    assert_eq!(iter.next(), Some((&"baboon".to_string(), &2)));
    assert_eq!(iter.next(), Some((&"coyote".to_string(), &3)));
    assert_eq!(iter.next(), None);
}

fn test_range() {
    let size = 200;
    // Miri is too slow
    let step = if cfg!(miri) { 66 } else { 1 };
    let map: BTreeMap<_, _> = (0..size).map(|i| (i, i)).collect();

    for i in (0..size).step_by(step) {
        for j in (i..size).step_by(step) {
            let mut kvs = map.range((Included(&i), Included(&j))).map(|(&k, &v)| (k, v));
            let mut pairs = (i..=j).map(|i| (i, i));

            for (kv, pair) in kvs.by_ref().zip(pairs.by_ref()) {
                assert_eq!(kv, pair);
            }
            assert_eq!(kvs.next(), None);
            assert_eq!(pairs.next(), None);
        }
    }
}

fn test_range_mut() {
    let size = 200;
    // Miri is too slow
    let step = if cfg!(miri) { 66 } else { 1 };
    let mut map: BTreeMap<_, _> = (0..size).map(|i| (i, i)).collect();

    for i in (0..size).step_by(step) {
        for j in (i..size).step_by(step) {
            let mut kvs = map.range_mut((Included(&i), Included(&j))).map(|(&k, &mut v)| (k, v));
            let mut pairs = (i..=j).map(|i| (i, i));

            for (kv, pair) in kvs.by_ref().zip(pairs.by_ref()) {
                assert_eq!(kv, pair);
            }
            assert_eq!(kvs.next(), None);
            assert_eq!(pairs.next(), None);
        }
    }
}

mod test_drain_filter {
    use super::*;

    pub fn empty() {
        let mut map: BTreeMap<i32, i32> = BTreeMap::new();
        map.drain_filter(|_, _| unreachable!("there's nothing to decide on"));
        assert!(map.is_empty());
    }

    pub fn consuming_nothing() {
        let pairs = (0..3).map(|i| (i, i));
        let mut map: BTreeMap<_, _> = pairs.collect();
        assert!(map.drain_filter(|_, _| false).eq(iter::empty()));
    }

    pub fn consuming_all() {
        let pairs = (0..3).map(|i| (i, i));
        let mut map: BTreeMap<_, _> = pairs.clone().collect();
        assert!(map.drain_filter(|_, _| true).eq(pairs));
    }

    pub fn mutating_and_keeping() {
        let pairs = (0..3).map(|i| (i, i));
        let mut map: BTreeMap<_, _> = pairs.collect();
        assert!(
            map.drain_filter(|_, v| {
                *v += 6;
                false
            })
            .eq(iter::empty())
        );
        assert!(map.keys().copied().eq(0..3));
        assert!(map.values().copied().eq(6..9));
    }

    pub fn mutating_and_removing() {
        let pairs = (0..3).map(|i| (i, i));
        let mut map: BTreeMap<_, _> = pairs.collect();
        assert!(
            map.drain_filter(|_, v| {
                *v += 6;
                true
            })
            .eq((0..3).map(|i| (i, i + 6)))
        );
        assert!(map.is_empty());
    }

    pub fn underfull_keeping_all() {
        let pairs = (0..3).map(|i| (i, i));
        let mut map: BTreeMap<_, _> = pairs.collect();
        map.drain_filter(|_, _| false);
        assert!(map.keys().copied().eq(0..3));
    }

    pub fn underfull_removing_one() {
        let pairs = (0..3).map(|i| (i, i));
        for doomed in 0..3 {
            let mut map: BTreeMap<_, _> = pairs.clone().collect();
            map.drain_filter(|i, _| *i == doomed);
            assert_eq!(map.len(), 2);
        }
    }

    pub fn underfull_keeping_one() {
        let pairs = (0..3).map(|i| (i, i));
        for sacred in 0..3 {
            let mut map: BTreeMap<_, _> = pairs.clone().collect();
            map.drain_filter(|i, _| *i != sacred);
            assert!(map.keys().copied().eq(sacred..=sacred));
        }
    }

    pub fn underfull_removing_all() {
        let pairs = (0..3).map(|i| (i, i));
        let mut map: BTreeMap<_, _> = pairs.collect();
        map.drain_filter(|_, _| true);
        assert!(map.is_empty());
    }

    pub fn height_0_keeping_all() {
        let pairs = (0..NODE_CAPACITY).map(|i| (i, i));
        let mut map: BTreeMap<_, _> = pairs.collect();
        map.drain_filter(|_, _| false);
        assert!(map.keys().copied().eq(0..NODE_CAPACITY));
    }

    pub fn height_0_removing_one() {
        let pairs = (0..NODE_CAPACITY).map(|i| (i, i));
        for doomed in 0..NODE_CAPACITY {
            let mut map: BTreeMap<_, _> = pairs.clone().collect();
            map.drain_filter(|i, _| *i == doomed);
            assert_eq!(map.len(), NODE_CAPACITY - 1);
        }
    }

    pub fn height_0_keeping_one() {
        let pairs = (0..NODE_CAPACITY).map(|i| (i, i));
        for sacred in 0..NODE_CAPACITY {
            let mut map: BTreeMap<_, _> = pairs.clone().collect();
            map.drain_filter(|i, _| *i != sacred);
            assert!(map.keys().copied().eq(sacred..=sacred));
        }
    }

    pub fn height_0_removing_all() {
        let pairs = (0..NODE_CAPACITY).map(|i| (i, i));
        let mut map: BTreeMap<_, _> = pairs.collect();
        map.drain_filter(|_, _| true);
        assert!(map.is_empty());
    }

    pub fn height_0_keeping_half() {
        let mut map: BTreeMap<_, _> = (0..16).map(|i| (i, i)).collect();
        assert_eq!(map.drain_filter(|i, _| *i % 2 == 0).count(), 8);
        assert_eq!(map.len(), 8);
    }

    pub fn height_1_removing_all() {
        let pairs = (0..MIN_INSERTS_HEIGHT_1).map(|i| (i, i));
        let mut map: BTreeMap<_, _> = pairs.collect();
        map.drain_filter(|_, _| true);
        assert!(map.is_empty());
    }

    pub fn height_1_removing_one() {
        let pairs = (0..MIN_INSERTS_HEIGHT_1).map(|i| (i, i));
        for doomed in 0..MIN_INSERTS_HEIGHT_1 {
            let mut map: BTreeMap<_, _> = pairs.clone().collect();
            map.drain_filter(|i, _| *i == doomed);
            assert_eq!(map.len(), MIN_INSERTS_HEIGHT_1 - 1);
        }
    }

    pub fn height_1_keeping_one() {
        let pairs = (0..MIN_INSERTS_HEIGHT_1).map(|i| (i, i));
        for sacred in 0..MIN_INSERTS_HEIGHT_1 {
            let mut map: BTreeMap<_, _> = pairs.clone().collect();
            map.drain_filter(|i, _| *i != sacred);
            assert!(map.keys().copied().eq(sacred..=sacred));
        }
    }

    pub fn height_2_removing_one() {
        let pairs = (0..MIN_INSERTS_HEIGHT_2).map(|i| (i, i));
        for doomed in (0..MIN_INSERTS_HEIGHT_2).step_by(12) {
            let mut map: BTreeMap<_, _> = pairs.clone().collect();
            map.drain_filter(|i, _| *i == doomed);
            assert_eq!(map.len(), MIN_INSERTS_HEIGHT_2 - 1);
        }
    }

    pub fn height_2_keeping_one() {
        let pairs = (0..MIN_INSERTS_HEIGHT_2).map(|i| (i, i));
        for sacred in (0..MIN_INSERTS_HEIGHT_2).step_by(12) {
            let mut map: BTreeMap<_, _> = pairs.clone().collect();
            map.drain_filter(|i, _| *i != sacred);
            assert!(map.keys().copied().eq(sacred..=sacred));
        }
    }

    pub fn height_2_removing_all() {
        let pairs = (0..MIN_INSERTS_HEIGHT_2).map(|i| (i, i));
        let mut map: BTreeMap<_, _> = pairs.collect();
        map.drain_filter(|_, _| true);
        assert!(map.is_empty());
    }

    pub fn drop_panic_leak() {
        static PREDS: AtomicUsize = AtomicUsize::new(0);
        static DROPS: AtomicUsize = AtomicUsize::new(0);

        struct D;
        impl Drop for D {
            fn drop(&mut self) {
                if DROPS.fetch_add(1, Ordering::SeqCst) == 1 {
                    panic!("panic in `drop`");
                }
            }
        }

        // Keys are multiples of 4, so that each key is counted by a hexadecimal digit.
        let mut map = (0..3).map(|i| (i * 4, D)).collect::<BTreeMap<_, _>>();

        catch_unwind(move || {
            drop(map.drain_filter(|i, _| {
                PREDS.fetch_add(1usize << i, Ordering::SeqCst);
                true
            }))
        })
        .unwrap_err();

        assert_eq!(PREDS.load(Ordering::SeqCst), 0x011);
        assert_eq!(DROPS.load(Ordering::SeqCst), 3);
    }

    pub fn pred_panic_leak() {
        static PREDS: AtomicUsize = AtomicUsize::new(0);
        static DROPS: AtomicUsize = AtomicUsize::new(0);

        struct D;
        impl Drop for D {
            fn drop(&mut self) {
                DROPS.fetch_add(1, Ordering::SeqCst);
            }
        }

        // Keys are multiples of 4, so that each key is counted by a hexadecimal digit.
        let mut map = (0..3).map(|i| (i * 4, D)).collect::<BTreeMap<_, _>>();

        catch_unwind(AssertUnwindSafe(|| {
            drop(map.drain_filter(|i, _| {
                PREDS.fetch_add(1usize << i, Ordering::SeqCst);
                match i {
                    0 => true,
                    _ => panic!(),
                }
            }))
        }))
        .unwrap_err();

        assert_eq!(PREDS.load(Ordering::SeqCst), 0x011);
        assert_eq!(DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(map.len(), 2);
        assert_eq!(map.first_entry().unwrap().key(), &4);
        assert_eq!(map.last_entry().unwrap().key(), &8);
    }

    // Same as above, but attempt to use the iterator again after the panic in the predicate
    pub fn pred_panic_reuse() {
        static PREDS: AtomicUsize = AtomicUsize::new(0);
        static DROPS: AtomicUsize = AtomicUsize::new(0);

        struct D;
        impl Drop for D {
            fn drop(&mut self) {
                DROPS.fetch_add(1, Ordering::SeqCst);
            }
        }

        // Keys are multiples of 4, so that each key is counted by a hexadecimal digit.
        let mut map = (0..3).map(|i| (i * 4, D)).collect::<BTreeMap<_, _>>();

        {
            let mut it = map.drain_filter(|i, _| {
                PREDS.fetch_add(1usize << i, Ordering::SeqCst);
                match i {
                    0 => true,
                    _ => panic!(),
                }
            });
            catch_unwind(AssertUnwindSafe(|| while it.next().is_some() {})).unwrap_err();
            // Iterator behaviour after a panic is explicitly unspecified,
            // so this is just the current implementation:
            let result = catch_unwind(AssertUnwindSafe(|| it.next()));
            assert!(matches!(result, Ok(None)));
        }

        assert_eq!(PREDS.load(Ordering::SeqCst), 0x011);
        assert_eq!(DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(map.len(), 2);
        assert_eq!(map.first_entry().unwrap().key(), &4);
        assert_eq!(map.last_entry().unwrap().key(), &8);
    }
}

fn test_borrow() {
    // make sure these compile -- using the Borrow trait
    {
        let mut map = BTreeMap::new();
        map.insert("0".to_string(), 1);
        assert_eq!(map["0"], 1);
    }

    {
        let mut map = BTreeMap::new();
        map.insert(Box::new(0), 1);
        assert_eq!(map[&0], 1);
    }

    {
        let mut map = BTreeMap::new();
        map.insert(Box::new([0, 1]) as Box<[i32]>, 1);
        assert_eq!(map[&[0, 1][..]], 1);
    }

    {
        let mut map = BTreeMap::new();
        map.insert(Rc::new(0), 1);
        assert_eq!(map[&0], 1);
    }
}

fn test_entry() {
    let xs = [(1, 10), (2, 20), (3, 30), (4, 40), (5, 50), (6, 60)];

    let mut map: BTreeMap<_, _> = xs.iter().cloned().collect();

    // Existing key (insert)
    match map.entry(1) {
        Vacant(_) => unreachable!(),
        Occupied(mut view) => {
            assert_eq!(view.get(), &10);
            assert_eq!(view.insert(100), 10);
        }
    }
    assert_eq!(map.get(&1).unwrap(), &100);
    assert_eq!(map.len(), 6);

    // Existing key (update)
    match map.entry(2) {
        Vacant(_) => unreachable!(),
        Occupied(mut view) => {
            let v = view.get_mut();
            *v *= 10;
        }
    }
    assert_eq!(map.get(&2).unwrap(), &200);
    assert_eq!(map.len(), 6);

    // Existing key (take)
    match map.entry(3) {
        Vacant(_) => unreachable!(),
        Occupied(view) => {
            assert_eq!(view.remove(), 30);
        }
    }
    assert_eq!(map.get(&3), None);
    assert_eq!(map.len(), 5);

    // Inexistent key (insert)
    match map.entry(10) {
        Occupied(_) => unreachable!(),
        Vacant(view) => {
            assert_eq!(*view.insert(1000), 1000);
        }
    }
    assert_eq!(map.get(&10).unwrap(), &1000);
    assert_eq!(map.len(), 6);
}

fn test_extend_ref() {
    let mut a = BTreeMap::new();
    a.insert(1, "one");
    let mut b = BTreeMap::new();
    b.insert(2, "two");
    b.insert(3, "three");

    a.extend(&b);

    assert_eq!(a.len(), 3);
    assert_eq!(a[&1], "one");
    assert_eq!(a[&2], "two");
    assert_eq!(a[&3], "three");
}

fn test_zst() {
    let mut m = BTreeMap::new();
    assert_eq!(m.len(), 0);

    assert_eq!(m.insert((), ()), None);
    assert_eq!(m.len(), 1);

    assert_eq!(m.insert((), ()), Some(()));
    assert_eq!(m.len(), 1);
    assert_eq!(m.iter().count(), 1);

    m.clear();
    assert_eq!(m.len(), 0);

    for _ in 0..100 {
        m.insert((), ());
    }

    assert_eq!(m.len(), 1);
    assert_eq!(m.iter().count(), 1);
}

// This test's only purpose is to ensure that zero-sized keys with nonsensical orderings
// do not cause segfaults when used with zero-sized values. All other map behavior is
// undefined.
fn test_bad_zst() {
    use std::cmp::Ordering;

    #[derive(Clone, Copy, Debug)]
    struct Bad;

    impl PartialEq for Bad {
        fn eq(&self, _: &Self) -> bool {
            false
        }
    }

    impl Eq for Bad {}

    impl PartialOrd for Bad {
        fn partial_cmp(&self, _: &Self) -> Option<Ordering> {
            Some(Ordering::Less)
        }
    }

    impl Ord for Bad {
        fn cmp(&self, _: &Self) -> Ordering {
            Ordering::Less
        }
    }

    let mut m = BTreeMap::new();

    for _ in 0..100 {
        m.insert(Bad, Bad);
    }
}

fn test_clone() {
    let mut map = BTreeMap::new();
    let size = MIN_INSERTS_HEIGHT_1;
    assert_eq!(map.len(), 0);

    for i in 0..size {
        assert_eq!(map.insert(i, 10 * i), None);
        assert_eq!(map.len(), i + 1);
        assert_eq!(map, map.clone());
    }

    for i in 0..size {
        assert_eq!(map.insert(i, 100 * i), Some(10 * i));
        assert_eq!(map.len(), size);
        assert_eq!(map, map.clone());
    }

    for i in 0..size / 2 {
        assert_eq!(map.remove(&(i * 2)), Some(i * 200));
        assert_eq!(map.len(), size - i - 1);
        assert_eq!(map, map.clone());
    }

    for i in 0..size / 2 {
        assert_eq!(map.remove(&(2 * i)), None);
        assert_eq!(map.remove(&(2 * i + 1)), Some(i * 200 + 100));
        assert_eq!(map.len(), size / 2 - i - 1);
        assert_eq!(map, map.clone());
    }

    // Test a tree with 2 semi-full levels and a tree with 3 levels.
    map = (1..MIN_INSERTS_HEIGHT_2).map(|i| (i, i)).collect();
    assert_eq!(map.len(), MIN_INSERTS_HEIGHT_2 - 1);
    assert_eq!(map, map.clone());
    map.insert(0, 0);
    assert_eq!(map.len(), MIN_INSERTS_HEIGHT_2);
    assert_eq!(map, map.clone());
}

fn test_clone_from() {
    let mut map1 = BTreeMap::new();
    let max_size = MIN_INSERTS_HEIGHT_1;

    // Range to max_size inclusive, because i is the size of map1 being tested.
    for i in 0..=max_size {
        let mut map2 = BTreeMap::new();
        for j in 0..i {
            let mut map1_copy = map2.clone();
            map1_copy.clone_from(&map1); // small cloned from large
            assert_eq!(map1_copy, map1);
            let mut map2_copy = map1.clone();
            map2_copy.clone_from(&map2); // large cloned from small
            assert_eq!(map2_copy, map2);
            map2.insert(100 * j + 1, 2 * j + 1);
        }
        map2.clone_from(&map1); // same length
        assert_eq!(map2, map1);
        map1.insert(i, 10 * i);
    }
}

fn test_occupied_entry_key() {
    let mut a = BTreeMap::new();
    let key = "hello there";
    let value = "value goes here";
    assert!(a.is_empty());
    a.insert(key.clone(), value.clone());
    assert_eq!(a.len(), 1);
    assert_eq!(a[key], value);

    match a.entry(key.clone()) {
        Vacant(_) => panic!(),
        Occupied(e) => assert_eq!(key, *e.key()),
    }
    assert_eq!(a.len(), 1);
    assert_eq!(a[key], value);
}

fn test_vacant_entry_key() {
    let mut a = BTreeMap::new();
    let key = "hello there";
    let value = "value goes here";

    assert!(a.is_empty());
    match a.entry(key.clone()) {
        Occupied(_) => panic!(),
        Vacant(e) => {
            assert_eq!(key, *e.key());
            e.insert(value.clone());
        }
    }
    assert_eq!(a.len(), 1);
    assert_eq!(a[key], value);
}

fn test_first_last_entry() {
    let mut a = BTreeMap::new();
    assert!(a.first_entry().is_none());
    assert!(a.last_entry().is_none());
    a.insert(1, 42);
    assert_eq!(a.first_entry().unwrap().key(), &1);
    assert_eq!(a.last_entry().unwrap().key(), &1);
    a.insert(2, 24);
    assert_eq!(a.first_entry().unwrap().key(), &1);
    assert_eq!(a.last_entry().unwrap().key(), &2);
    a.insert(0, 6);
    assert_eq!(a.first_entry().unwrap().key(), &0);
    assert_eq!(a.last_entry().unwrap().key(), &2);
    let (k1, v1) = a.first_entry().unwrap().remove_entry();
    assert_eq!(k1, 0);
    assert_eq!(v1, 6);
    let (k2, v2) = a.last_entry().unwrap().remove_entry();
    assert_eq!(k2, 2);
    assert_eq!(v2, 24);
    assert_eq!(a.first_entry().unwrap().key(), &1);
    assert_eq!(a.last_entry().unwrap().key(), &1);
}

fn test_insert_into_full_left() {
    let mut map: BTreeMap<_, _> = (0..NODE_CAPACITY).map(|i| (i * 2, ())).collect();
    assert!(map.insert(NODE_CAPACITY, ()).is_none());
}

fn test_insert_into_full_right() {
    let mut map: BTreeMap<_, _> = (0..NODE_CAPACITY).map(|i| (i * 2, ())).collect();
    assert!(map.insert(NODE_CAPACITY + 2, ()).is_none());
}

macro_rules! create_append_test {
    ($name:ident, $len:expr) => {
        fn $name() {
            let mut a = BTreeMap::new();
            for i in 0..8 {
                a.insert(i, i);
            }

            let mut b = BTreeMap::new();
            for i in 5..$len {
                b.insert(i, 2 * i);
            }

            a.append(&mut b);

            assert_eq!(a.len(), $len);
            assert_eq!(b.len(), 0);

            for i in 0..$len {
                if i < 5 {
                    assert_eq!(a[&i], i);
                } else {
                    assert_eq!(a[&i], 2 * i);
                }
            }

            assert_eq!(a.remove(&($len - 1)), Some(2 * ($len - 1)));
            assert_eq!(a.insert($len - 1, 20), None);
        }
    };
}

// These are mostly for testing the algorithm that "fixes" the right edge after insertion.
// Single node.
create_append_test!(test_append_9, 9);
// Two leafs that don't need fixing.
create_append_test!(test_append_17, 17);
// Two leafs where the second one ends up underfull and needs stealing at the end.
create_append_test!(test_append_14, 14);
// Two leafs where the second one ends up empty because the insertion finished at the root.
create_append_test!(test_append_12, 12);
// Three levels; insertion finished at the root.
create_append_test!(test_append_144, 144);
// Three levels; insertion finished at leaf while there is an empty node on the second level.
create_append_test!(test_append_145, 145);
// Tests for several randomly chosen sizes.
create_append_test!(test_append_170, 170);
create_append_test!(test_append_181, 181);
#[cfg(not(miri))] // Miri is too slow
create_append_test!(test_append_239, 239);
#[cfg(not(miri))] // Miri is too slow
create_append_test!(test_append_1700, 1700);

fn rand_data(len: usize) -> Vec<(u32, u32)> {
    let mut rng = DeterministicRng::new();
    Vec::from_iter((0..len).map(|_| (rng.next(), rng.next())))
}

fn test_split_off_empty_right() {
    let mut data = rand_data(173);

    let mut map = BTreeMap::from_iter(data.clone());
    let right = map.split_off(&(data.iter().max().unwrap().0 + 1));

    data.sort();
    assert!(map.into_iter().eq(data));
    assert!(right.into_iter().eq(None));
}

fn test_split_off_empty_left() {
    let mut data = rand_data(314);

    let mut map = BTreeMap::from_iter(data.clone());
    let right = map.split_off(&data.iter().min().unwrap().0);

    data.sort();
    assert!(map.into_iter().eq(None));
    assert!(right.into_iter().eq(data));
}

// In a tree with 3 levels, if all but a part of the first leaf node is split off,
// make sure fix_top eliminates both top levels.
fn test_split_off_tiny_left_height_2() {
    let pairs = (0..MIN_INSERTS_HEIGHT_2).map(|i| (i, i));
    let mut left: BTreeMap<_, _> = pairs.clone().collect();
    let right = left.split_off(&1);
    assert_eq!(left.len(), 1);
    assert_eq!(right.len(), MIN_INSERTS_HEIGHT_2 - 1);
    assert_eq!(*left.first_key_value().unwrap().0, 0);
    assert_eq!(*right.first_key_value().unwrap().0, 1);
}

// In a tree with 3 levels, if only part of the last leaf node is split off,
// make sure fix_top eliminates both top levels.
fn test_split_off_tiny_right_height_2() {
    let pairs = (0..MIN_INSERTS_HEIGHT_2).map(|i| (i, i));
    let last = MIN_INSERTS_HEIGHT_2 - 1;
    let mut left: BTreeMap<_, _> = pairs.clone().collect();
    assert_eq!(*left.last_key_value().unwrap().0, last);
    let right = left.split_off(&last);
    assert_eq!(left.len(), MIN_INSERTS_HEIGHT_2 - 1);
    assert_eq!(right.len(), 1);
    assert_eq!(*left.last_key_value().unwrap().0, last - 1);
    assert_eq!(*right.last_key_value().unwrap().0, last);
}

fn test_split_off_large_random_sorted() {
    // Miri is too slow
    let mut data = if cfg!(miri) { rand_data(529) } else { rand_data(1529) };
    // special case with maximum height.
    data.sort();

    let mut map = BTreeMap::from_iter(data.clone());
    let key = data[data.len() / 2].0;
    let right = map.split_off(&key);

    assert!(map.into_iter().eq(data.clone().into_iter().filter(|x| x.0 < key)));
    assert!(right.into_iter().eq(data.into_iter().filter(|x| x.0 >= key)));
}

fn test_into_iter_drop_leak_height_0() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);

    struct D;

    impl Drop for D {
        fn drop(&mut self) {
            if DROPS.fetch_add(1, Ordering::SeqCst) == 3 {
                panic!("panic in `drop`");
            }
        }
    }

    let mut map = BTreeMap::new();
    map.insert("a", D);
    map.insert("b", D);
    map.insert("c", D);
    map.insert("d", D);
    map.insert("e", D);

    catch_unwind(move || drop(map.into_iter())).unwrap_err();

    assert_eq!(DROPS.load(Ordering::SeqCst), 5);
}

fn test_into_iter_drop_leak_height_1() {
    let size = MIN_INSERTS_HEIGHT_1;
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    static PANIC_POINT: AtomicUsize = AtomicUsize::new(0);

    struct D;
    impl Drop for D {
        fn drop(&mut self) {
            if DROPS.fetch_add(1, Ordering::SeqCst) == PANIC_POINT.load(Ordering::SeqCst) {
                panic!("panic in `drop`");
            }
        }
    }

    for panic_point in vec![0, 1, size - 2, size - 1] {
        DROPS.store(0, Ordering::SeqCst);
        PANIC_POINT.store(panic_point, Ordering::SeqCst);
        let map: BTreeMap<_, _> = (0..size).map(|i| (i, D)).collect();
        catch_unwind(move || drop(map.into_iter())).unwrap_err();
        assert_eq!(DROPS.load(Ordering::SeqCst), size);
    }
}

fn test_into_keys() {
    let vec = vec![(1, 'a'), (2, 'b'), (3, 'c')];
    let map: BTreeMap<_, _> = vec.into_iter().collect();
    let keys: Vec<_> = map.into_keys().collect();

    assert_eq!(keys.len(), 3);
    assert!(keys.contains(&1));
    assert!(keys.contains(&2));
    assert!(keys.contains(&3));
}

fn test_into_values() {
    let vec = vec![(1, 'a'), (2, 'b'), (3, 'c')];
    let map: BTreeMap<_, _> = vec.into_iter().collect();
    let values: Vec<_> = map.into_values().collect();

    assert_eq!(values.len(), 3);
    assert!(values.contains(&'a'));
    assert!(values.contains(&'b'));
    assert!(values.contains(&'c'));
}

fn test_insert_remove_intertwined() {
    let loops = if cfg!(miri) { 100 } else { 1_000_000 };
    let mut map = BTreeMap::new();
    let mut i = 1;
    for _ in 0..loops {
        i = (i + 421) & 0xFF;
        map.insert(i, i);
        map.remove(&(0xFF - i));
    }
}

fn main() {
    test_basic_large();
    test_basic_small();
    test_iter();
    test_iter_rev();
    test_iter_mut_mutation();
    test_values_mut();
    test_values_mut_mutation();
    test_iter_entering_root_twice();
    test_iter_descending_to_same_node_twice();
    test_iter_mixed();
    test_iter_min_max();
    test_range_small();
    test_range_height_1();
    test_range_large();
    test_range_inclusive_max_value();
    test_range_equal_empty_cases();
    //[should_panic] test_range_equal_excluded();
    //[should_panic] test_range_backwards_1();
    //[should_panic] test_range_backwards_2();
    //[should_panic] test_range_backwards_3();
    //[should_panic] test_range_backwards_4();
    test_range_1000();
    test_range_borrowed_key();
    test_range();
    test_range_mut();
    test_drain_filter::empty();
    test_drain_filter::consuming_nothing();
    test_drain_filter::consuming_all();
    test_drain_filter::mutating_and_keeping();
    test_drain_filter::mutating_and_removing();
    test_drain_filter::underfull_keeping_all();
    test_drain_filter::underfull_removing_one();
    test_drain_filter::underfull_keeping_one();
    test_drain_filter::underfull_removing_all();
    test_drain_filter::height_0_keeping_all();
    test_drain_filter::height_0_removing_one();
    test_drain_filter::height_0_keeping_one();
    test_drain_filter::height_0_removing_all();
    test_drain_filter::height_0_keeping_half();
    test_drain_filter::height_1_removing_all();
    test_drain_filter::height_1_removing_one();
    test_drain_filter::height_1_keeping_one();
    test_drain_filter::height_2_removing_one();
    test_drain_filter::height_2_keeping_one();
    test_drain_filter::height_2_removing_all();
    // involves panic test_drain_filter::drop_panic_leak();
    // involves panic test_drain_filter::pred_panic_leak();
    // involves panic test_drain_filter::pred_panic_reuse();
    test_borrow();
    test_entry();
    test_extend_ref();
    test_zst();
    test_bad_zst();
    test_clone();
    test_clone_from();
    test_occupied_entry_key();
    test_vacant_entry_key();
    test_first_last_entry();
    test_insert_into_full_left();
    test_insert_into_full_right();
    test_append_9();
    test_append_17();
    test_append_14();
    test_append_12();
    test_append_144();
    test_append_145();
    test_append_170();
    test_append_181();
    // involves Vec::sort test_split_off_empty_right();
    // involves Vec::sort test_split_off_empty_left();
    test_split_off_tiny_left_height_2();
    test_split_off_tiny_right_height_2();
    // involves Vec::sort test_split_off_large_random_sorted();
    // involves panic test_into_iter_drop_leak_height_0();
    // involves panic test_into_iter_drop_leak_height_1();
    test_into_keys();
    test_into_values();
    test_insert_remove_intertwined();
}
