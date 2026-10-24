pub(crate) fn next(c: &mut [usize], n: usize) -> bool {
    let k = c.len();
    let Some(i) = (0..k).rev().find(|&i| c[i] != i + n - k) else {
        return false;
    };
    c[i] += 1;
    for j in i + 1..k {
        c[j] = c[j - 1] + 1;
    }
    true
}

pub(crate) fn for_each(n: usize, size: usize, mut f: impl FnMut(&[usize])) {
    if size > n {
        return;
    }
    let mut indices: Vec<usize> = (0..size).collect();
    loop {
        f(&indices);
        if !next(&mut indices, n) {
            break;
        }
    }
}
