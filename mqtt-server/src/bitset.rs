#[derive(Debug)]
pub(crate) struct BitSet<const N: usize>
where
    [(); (N + 31) / 32]: Sized,
{
    set: [u32; (N + 31) / 32],
}

impl<const N: usize> Default for BitSet<N>
where
    [(); (N + 31) / 32]: Sized,
{
    fn default() -> Self {
        Self {
            set: [0; { (N + 31) / 32 }],
        }
    }
}

impl<const N: usize> BitSet<N>
where
    [(); (N + 31) / 32]: Sized,
{
    pub fn set(&mut self, index: usize) {
        assert!(index < N, "index out of bounds");
        let word = index / 32;
        let bit = index % 32;
        self.set[word] |= 1 << bit;
    }

    pub fn unset(&mut self, index: usize) {
        assert!(index < N, "index out of bounds");
        let word = index / 32;
        let bit = index % 32;
        self.set[word] &= !(1 << bit);
    }

    pub fn get(&self, index: usize) -> bool {
        assert!(index < N, "index out of bounds");
        let word = index / 32;
        let bit = index % 32;
        self.set[word] & (1 << bit) != 0
    }

    #[allow(dead_code)]
    pub fn iter_ones(&self) -> impl Iterator<Item = usize> + '_ {
        self.set.iter().enumerate().flat_map(|(i, &word)| {
            (0..32).filter_map(move |j| {
                if word & (1 << j) != 0 {
                    // todo check if i * 32 + j >= N
                    Some(i * 32 + j)
                } else {
                    None
                }
            })
        })
    }

    pub fn count_ones(&self) -> usize {
        // todo ignore last bits that are > N
        self.set.iter().map(|word| word.count_ones() as usize).sum()
    }
    pub fn is_empty(&self) -> bool {
        self.set.iter().all(|&word| word == 0)
    }
}
