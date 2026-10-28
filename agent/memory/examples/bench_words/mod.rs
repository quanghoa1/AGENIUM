//! What the memory benchmarks share: a seeded random generator and the vocabulary their synthetic facts are made of
//! (`memory_bench`, `ann_bench`), so both measure on the same words.

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A 6 000-word vocabulary (some real engineering words first, then pronounceable pseudo-words) and its Zipf CDF.
pub fn vocabulary() -> (Vec<String>, Vec<f64>) {
    let real = "the a to of and in for is on with run test build file config error fix deploy merge commit branch \
        ticket agent skill memory lesson review report api server client request response timeout retry cache index \
        query table column migration schema backup restore token secret key vault sandbox shell command path network \
        project team lead worker plan spec acceptance criterion evidence proof diff patch release version tag log";
    let mut v: Vec<String> = real.split_whitespace().map(str::to_owned).collect();
    let syll = [
        "ka", "ro", "mi", "tu", "sel", "dan", "ver", "lo", "pri", "zen", "ox", "qua", "bel", "tor",
        "nis",
    ];
    let mut r = Rng(0x9E37_79B9_7F4A_7C15);
    while v.len() < 6000 {
        let n = 2 + (r.next() % 3) as usize;
        let w: String = (0..n)
            .map(|_| syll[(r.next() % syll.len() as u64) as usize])
            .collect();
        if !v.contains(&w) {
            v.push(w);
        }
    }
    let weights: Vec<f64> = (1..=v.len()).map(|i| 1.0 / (i as f64).powf(1.1)).collect();
    let total: f64 = weights.iter().sum();
    let mut acc = 0.0;
    let cdf = weights.iter().map(|w| {
        acc += w / total;
        acc
    });
    (v, cdf.collect())
}
