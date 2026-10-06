//! A suffix array over the normalized token stream, and the repeated runs in it.
//!
//! This is the `CCFinder` method. Every file's leaves become one token
//! sequence (identifiers collapsed to a single token, literals to their kind),
//! files are separated by sentinels no other position shares, and the suffix
//! array plus its longest-common-prefix array enumerate every repeated run in
//! one linear scan. Because the runs ignore syntax entirely, they find copies
//! that straddle the boundaries the fragment-based detectors look at — the
//! tail of one function and the head of the next, or six statements out of a
//! longer block.

/// One maximal repeated run: `length` tokens starting at each of `starts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Repeat {
    /// Positions in the concatenated sequence.
    pub starts: Vec<u32>,
    /// Tokens in the run.
    pub length: u32,
}

/// The suffix array of `text`, by prefix doubling with radix-sorted ranks.
///
/// `O(n log n)`. The input is expected to end each file with a unique
/// sentinel, which bounds the number of doubling rounds by the length of the
/// longest file rather than of the whole repository.
#[must_use]
pub(crate) fn suffix_array(text: &[u32]) -> Vec<u32> {
    let n = text.len();
    if n == 0 {
        return Vec::new();
    }

    let mut alphabet: Vec<u32> = text.to_vec();
    alphabet.sort_unstable();
    alphabet.dedup();
    let mut rank: Vec<u32> = text
        .iter()
        .map(|token| to_u32(alphabet.binary_search(token).unwrap_or_default()))
        .collect();

    let mut sa: Vec<u32> = (0..to_u32(n)).collect();
    sa.sort_unstable_by_key(|&position| rank[position as usize]);

    let mut next_rank = vec![0_u32; n];
    let mut by_second = Vec::with_capacity(n);
    let mut counts: Vec<u32> = Vec::new();
    let mut step = 1_usize;

    loop {
        // Order by the second half of each key: suffixes too short to have
        // one sort first, then the rest in the order of the previous round.
        by_second.clear();
        by_second.extend(to_u32(n.saturating_sub(step))..to_u32(n));
        by_second.extend(
            sa.iter()
                .filter(|&&position| position as usize >= step)
                .map(|&position| to_u32(position as usize - step)),
        );

        // A stable counting sort by the first half finishes the round.
        let classes = rank.iter().max().map_or(1, |&max| max as usize + 1);
        counts.clear();
        counts.resize(classes + 1, 0);
        for &position in &by_second {
            counts[rank[position as usize] as usize + 1] += 1;
        }
        for class in 1..counts.len() {
            counts[class] += counts[class - 1];
        }
        for &position in &by_second {
            let slot = &mut counts[rank[position as usize] as usize];
            sa[*slot as usize] = position;
            *slot += 1;
        }

        let key = |position: u32| {
            let position = position as usize;
            let second = rank.get(position + step).map_or(0, |&value| value + 1);
            (rank[position], second)
        };
        next_rank[sa[0] as usize] = 0;
        for index in 1..n {
            let previous = next_rank[sa[index - 1] as usize];
            let bump = u32::from(key(sa[index - 1]) != key(sa[index]));
            next_rank[sa[index] as usize] = previous + bump;
        }
        std::mem::swap(&mut rank, &mut next_rank);

        if rank[sa[n - 1] as usize] as usize == n - 1 || step >= n {
            break;
        }
        step *= 2;
    }

    sa
}

/// Kasai's algorithm: `lcp[i]` is the common prefix of `sa[i - 1]` and `sa[i]`.
#[must_use]
pub(crate) fn lcp_array(text: &[u32], sa: &[u32]) -> Vec<u32> {
    let n = text.len();
    let mut rank = vec![0_usize; n];
    for (index, &position) in sa.iter().enumerate() {
        rank[position as usize] = index;
    }

    let mut lcp = vec![0_u32; n];
    let mut matched = 0_usize;
    for position in 0..n {
        if rank[position] == 0 {
            matched = 0;
            continue;
        }
        let other = sa[rank[position] - 1] as usize;
        while position + matched < n
            && other + matched < n
            && text[position + matched] == text[other + matched]
        {
            matched += 1;
        }
        lcp[rank[position]] = to_u32(matched);
        matched = matched.saturating_sub(1);
    }

    lcp
}

/// Every left-maximal repeated run of at least `min_length` tokens.
///
/// Walks the LCP intervals bottom-up. An interval whose occurrences are all
/// preceded by the same token is skipped: it is the tail of a longer run that
/// is reported on its own. At most `max_occurrences` positions are kept per
/// run, so one boilerplate line repeated a thousand times costs a thousand
/// tokens of work rather than a million.
#[must_use]
pub(crate) fn repeats(
    text: &[u32],
    sa: &[u32],
    lcp: &[u32],
    min_length: u32,
    max_occurrences: usize,
) -> Vec<Repeat> {
    let n = sa.len();
    let mut found = Vec::new();
    let mut stack: Vec<(u32, usize)> = vec![(0, 0)];

    for index in 1..=n {
        let current = if index < n { lcp[index] } else { 0 };
        let mut left = index - 1;
        while let Some(&(length, bound)) = stack.last() {
            if current >= length {
                break;
            }
            stack.pop();
            left = bound;
            if length >= min_length {
                let starts = &sa[bound..index];
                if is_left_maximal(text, starts) {
                    let mut starts: Vec<u32> =
                        starts.iter().copied().take(max_occurrences).collect();
                    starts.sort_unstable();
                    found.push(Repeat { starts, length });
                }
            }
        }
        if stack.last().is_none_or(|&(length, _)| current > length) {
            stack.push((current, left));
        }
    }

    found
}

/// Whether the occurrences do not all extend one token to the left.
fn is_left_maximal(text: &[u32], starts: &[u32]) -> bool {
    let mut previous = None;
    for &start in starts {
        let Some(before) = (start as usize).checked_sub(1).map(|index| text[index]) else {
            return true;
        };
        match previous {
            None => previous = Some(before),
            Some(seen) if seen != before => return true,
            Some(_) => {}
        }
    }
    false
}

/// Narrows an index to `u32`; token streams are far below four billion.
fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
