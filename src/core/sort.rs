//! JavaScriptCore's comparison-sort schedule, needed because compareNodes is not
//! a total order (phase ordering and false versus absent archived flags).
//! Algorithm: oven-sh/WebKit, Source/JavaScriptCore/runtime/StableSort.h.

fn insertion<Element: Copy>(values: &mut [Element], sorted_header: usize, less: &impl Fn(Element, Element) -> bool) {
    for index in sorted_header + 1..values.len() {
        let value = values[index];
        let (mut left, mut right) = (0, index);
        while left < right {
            let middle = left + (right - left) / 2;
            if less(value, values[middle]) {
                right = middle;
            } else {
                left = middle + 1;
            }
        }
        values[left..=index].rotate_right(1);
    }
}

fn run<Element: Copy>(values: &mut [Element], begin: usize, less: &impl Fn(Element, Element) -> bool) -> usize {
    let mut end = begin;
    if end + 1 < values.len() {
        let descending = less(values[end + 1], values[end]);
        end += 1;
        while end + 1 < values.len() && less(values[end + 1], values[end]) == descending {
            end += 1;
        }
        if descending {
            values[begin..=end].reverse();
        }
    }
    if end - begin < 8 {
        let length = 64.min(values.len() - begin);
        insertion(&mut values[begin..begin + length], end - begin, less);
        end = begin + length - 1;
    }
    while end + 1 < values.len() && !less(values[end + 1], values[end]) {
        end += 1;
    }
    end
}

// The exponential probes matter for Bun parity with a non-total comparator.
fn gallop<Element: Copy>(
    key: Element,
    values: &[Element],
    hint: usize,
    after_equal: bool,
    less: &impl Fn(Element, Element) -> bool,
) -> usize {
    let before = |value| if after_equal { !less(key, value) } else { less(value, key) };
    let (mut previous, mut offset) = (0, 1);
    let (mut left, mut right);
    if before(values[hint]) {
        let maximum = values.len() - hint;
        while offset < maximum && before(values[hint + offset]) {
            previous = offset;
            offset = (offset * 2 + 1).min(maximum);
        }
        left = hint + previous + 1;
        right = hint + offset.min(maximum);
    } else {
        let maximum = hint + 1;
        while offset < maximum && !before(values[hint - offset]) {
            previous = offset;
            offset = (offset * 2 + 1).min(maximum);
        }
        left = hint + 1 - offset.min(maximum);
        right = hint - previous;
    }
    while left < right {
        let middle = left + (right - left) / 2;
        if before(values[middle]) {
            left = middle + 1;
        } else {
            right = middle;
        }
    }
    right
}

#[derive(Clone, Copy)]
struct Run {
    begin: usize,
    end: usize,
}

fn merge<Element: Copy>(
    values: &mut [Element],
    first: Run,
    second: Run,
    minimum_gallop: &mut usize,
    less: &impl Fn(Element, Element) -> bool,
) {
    let source = values.to_vec();
    let mut left = first.begin;
    let left_end = first.end + 1;
    let mut right = second.begin;
    let mut right_end = second.end + 1;
    left += gallop(source[right], &source[left..left_end], 0, true, less);
    if left == left_end {
        return;
    }
    let right_length = right_end - right;
    right_end = right + gallop(source[left_end - 1], &source[right..right_end], right_length - 1, false, less);
    if right == right_end {
        return;
    }
    let mut destination = left;
    'merging: loop {
        let (mut left_wins, mut right_wins) = (0, 0);
        while left_wins < *minimum_gallop && right_wins < *minimum_gallop {
            if left == left_end || right == right_end {
                break 'merging;
            }
            if less(source[right], source[left]) {
                values[destination] = source[right];
                right += 1;
                right_wins += 1;
                left_wins = 0;
            } else {
                values[destination] = source[left];
                left += 1;
                left_wins += 1;
                right_wins = 0;
            }
            destination += 1;
        }
        *minimum_gallop += 1;
        loop {
            *minimum_gallop = (*minimum_gallop - 1).max(1);
            if left == left_end || right == right_end {
                break 'merging;
            }
            left_wins = gallop(source[right], &source[left..left_end], 0, true, less);
            values[destination..destination + left_wins].copy_from_slice(&source[left..left + left_wins]);
            destination += left_wins;
            left += left_wins;
            values[destination] = source[right];
            destination += 1;
            right += 1;
            if left == left_end || right == right_end {
                break 'merging;
            }
            right_wins = gallop(source[left], &source[right..right_end], 0, false, less);
            values[destination..destination + right_wins].copy_from_slice(&source[right..right + right_wins]);
            destination += right_wins;
            right += right_wins;
            values[destination] = source[left];
            destination += 1;
            left += 1;
            if left == left_end || right == right_end {
                break 'merging;
            }
            if left_wins < 7 && right_wins < 7 {
                break;
            }
        }
        *minimum_gallop += 1;
    }
    values[destination..destination + left_end - left].copy_from_slice(&source[left..left_end]);
    destination += left_end - left;
    values[destination..destination + right_end - right].copy_from_slice(&source[right..right_end]);
}

pub(super) fn bun_sort<Element: Copy>(values: &mut [Element], less: impl Fn(Element, Element) -> bool) {
    if values.len() < 8 {
        insertion(values, 0, &less);
        return;
    }
    let mut first = Run {
        begin: 0,
        end: run(values, 0, &less),
    };
    let mut stack: Vec<(Run, u32)> = vec![];
    let mut minimum_gallop = 7;
    while first.end + 1 < values.len() {
        let begin = first.end + 1;
        let second = Run {
            begin,
            end: run(values, begin, &less),
        };
        let left_midpoint = (first.begin as u128 + second.begin as u128) << 62;
        let right_midpoint = (second.begin as u128 + second.end as u128 + 1) << 62;
        let difference = (left_midpoint / values.len() as u128) ^ (right_midpoint / values.len() as u128);
        let power = (difference as u64).leading_zeros();
        while stack.last().is_some_and(|entry| entry.1 > power) {
            let previous = stack.pop().expect("pending run").0;
            merge(values, previous, first, &mut minimum_gallop, &less);
            first.begin = previous.begin;
        }
        stack.push((first, power));
        first = second;
    }
    while let Some((previous, _)) = stack.pop() {
        merge(values, previous, first, &mut minimum_gallop, &less);
        first.begin = previous.begin;
    }
}
