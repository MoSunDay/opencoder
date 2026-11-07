//! f32 embedding blob codec and brute-force cosine top-k.
//!
//! Embeddings are stored as little-endian f32 blobs (4 bytes per
//! dimension); search is a full scan plus bounded-size selection, which is
//! exact for the catalog sizes the capability library holds.
//!
//! Similarity is only defined between vectors of the SAME dimension whose
//! values are all finite: `cosine` answers `None` otherwise and `top_k`
//! skips such candidates, so a torn blob or a mismatched dimension is
//! never silently truncated into a score.

/// Encode an embedding as a little-endian f32 blob.
pub fn encode_f32_le(values: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Decode a little-endian f32 blob; trailing partial dimensions are
/// dropped (a torn write can never produce a valid vector anyway — pair
/// it with [`decode_f32_le_exact`] wherever a declared dimension applies).
pub fn decode_f32_le(blob: &[u8]) -> Vec<f32> {
    let (chunks, _) = blob.as_chunks::<4>();
    chunks
        .iter()
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Decode exactly `dim` finite f32 values, or `None`.
///
/// The strict counterpart of [`decode_f32_le`]: the blob must be exactly
/// `dim * 4` bytes and every decoded value finite. Torn, over-long or
/// non-finite blobs answer `None` instead of decoding into a shorter
/// vector that search could score as a valid lower-dimensional one.
pub fn decode_f32_le_exact(blob: &[u8], dim: usize) -> Option<Vec<f32>> {
    if blob.len() != dim.checked_mul(4)? {
        return None;
    }
    let values = decode_f32_le(blob);
    values.iter().all(|v| v.is_finite()).then_some(values)
}

/// Cosine similarity in [-1, 1]; zero vectors map to 0 (no direction).
///
/// `None` means "not comparable": the lengths differ (dimensions must
/// match exactly — no silent truncation to the shorter side) or either
/// vector holds non-finite values (NaN/Inf have no similarity).
pub fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || !a.iter().chain(b).all(|v| v.is_finite()) {
        return None;
    }
    let (mut dot, mut na, mut nb) = (0f32, 0f32, 0f32);
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom == 0.0 {
        Some(0.0)
    } else {
        // Finite inputs can still overflow the accumulators to inf; a
        // non-finite score is not a score.
        let score = dot / denom;
        score.is_finite().then_some(score)
    }
}

/// The `k` highest-similarity candidates, best first.
///
/// Candidates whose similarity is undefined ([`cosine`] answers `None`:
/// dimension mismatch with the query, or non-finite values) are skipped
/// rather than scored, so a corrupt row never takes a slot from a valid
/// one. Ties keep the earlier candidate (stable ordering by input
/// position). `k == 0` ranks nothing.
pub fn top_k<'a>(
    candidates: impl IntoIterator<Item = (&'a str, &'a [f32])>,
    query: &[f32],
    k: usize,
) -> Vec<(String, f32)> {
    if k == 0 {
        return Vec::new();
    }
    // Bounded insertion into a worst-first list: equivalent to sorting all
    // scores but never materializes more than k entries.
    let mut best: Vec<(String, f32)> = Vec::with_capacity(k.min(16));
    for (id, vec) in candidates {
        let Some(score) = cosine(vec, query) else {
            continue;
        };
        if best.len() < k {
            best.push((id.to_string(), score));
            best.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap_or(std::cmp::Ordering::Equal));
        } else if score > best[0].1 {
            best[0] = (id.to_string(), score);
            best.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap_or(std::cmp::Ordering::Equal));
        }
    }
    best.reverse();
    best
}
