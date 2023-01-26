use std::ops::Range;

#[derive(Debug)]
struct Elem<T> {
    range: Range<u64>,
    value: T,
}

#[derive(Debug)]
pub struct PhysAlloc<T> {
    allocations: Vec<Elem<T>>,
}

#[derive(Debug)]
pub struct AllocError<'a, T> {
    pub allocation: (Range<u64>, &'a T),
    pub value: T,
}

impl<T> PhysAlloc<T> {
    pub fn get(&self, range: Range<u64>) -> Option<(Range<u64>, &T)> {
        self.allocations
            .iter()
            .find(|elem| elem.range.overlaps(&range))
            .map(|elem| (elem.range.clone(), &elem.value))
    }

    pub fn alloc(&mut self, range: Range<u64>, value: T) -> Result<(), AllocError<'_, T>> {
        match self.get(range.clone()) {
            Some(_) => Err(AllocError { allocation: self.get(range).unwrap(), value }),
            None => {
                self.allocations.push(Elem { range, value });
                Ok(())
            }
        }
    }
}

impl<T> Default for PhysAlloc<T> {
    fn default() -> Self {
        Self { allocations: Default::default() }
    }
}

trait RangeExt {
    fn overlaps(&self, other: &Self) -> bool;
}

impl RangeExt for std::ops::Range<u64> {
    fn overlaps(&self, other: &Self) -> bool {
        self.start.max(other.start) <= self.end.min(other.end)
    }
}
