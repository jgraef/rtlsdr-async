use std::{
    cmp::Ordering,
    fmt::Debug,
    ops::{
        Index,
        IndexMut,
    },
};

#[derive(Clone)]
pub struct SparseList<T> {
    items: Vec<Option<T>>,
    free_list: Vec<usize>,
}

impl<T> Default for SparseList<T> {
    fn default() -> Self {
        Self {
            items: vec![],
            free_list: vec![],
        }
    }
}

impl<T> SparseList<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, value: T) -> usize {
        if let Some(index) = self.free_list.pop() {
            assert!(self.items[index].is_none());
            self.items[index] = Some(value);
            index
        }
        else {
            let index = self.items.len();
            self.items.push(Some(value));
            index
        }
    }

    pub fn insert_and_get_mut(&mut self, value: T) -> (usize, &mut T) {
        let index = self.insert(value);
        (index, &mut self[index])
    }

    pub fn reserve(&mut self) -> usize {
        if let Some(index) = self.free_list.pop() {
            assert!(self.items[index].is_none());
            index
        }
        else {
            let index = self.items.len();
            index
        }
    }

    pub fn insert_reserved(&mut self, index: usize, value: T) {
        match index.cmp(&self.items.len()) {
            Ordering::Less => {
                assert!(
                    self.items[index].is_none(),
                    "reserved index is already full"
                );
                self.items[index] = Some(value);
            }
            Ordering::Equal => {
                self.items.push(Some(value));
            }
            Ordering::Greater => panic!("reserved index is invalid"),
        }
    }

    pub fn remove(&mut self, index: usize) -> Option<T> {
        let value = std::mem::take(&mut self.items[index]);
        if value.is_some() {
            self.free_list.push(index);
        }
        value
    }

    pub fn iter(&self) -> Iter<'_, T> {
        Iter {
            iter: self.items.iter().filter_map(|item| item.as_ref()),
        }
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.items.get(index).and_then(|item| item.as_ref())
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.items.get_mut(index).and_then(|item| item.as_mut())
    }
}

impl<T: Debug> Debug for SparseList<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T> Index<usize> for SparseList<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index)
            .unwrap_or_else(|| panic!("invalid index: {index}"))
    }
}

impl<T> IndexMut<usize> for SparseList<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.get_mut(index)
            .unwrap_or_else(|| panic!("invalid index: {index}"))
    }
}

pub struct Iter<'a, T> {
    iter: std::iter::FilterMap<std::slice::Iter<'a, Option<T>>, fn(&Option<T>) -> Option<&T>>,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}
