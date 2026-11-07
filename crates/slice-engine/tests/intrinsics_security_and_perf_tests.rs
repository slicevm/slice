use crate::boa_realm::BOA_KV_MEM_BUDGET;
use crate::intrinsics::{
    aes_cbc_decrypt_with_expanded, aes_cbc_encrypt_with_expanded, base64_decode, base64_encode,
    crc32_checksum, ed25519_keypair_from_seed, ed25519_seed_and_pub_to_pkcs8_v2,
    ed25519_seed_to_pkcs8_v1, extract_ed25519_seed, fill_random_bytes, generate_uuid_v4,
    hkdf_derive_bytes, parse_whatwg_url, pbkdf2_derive_bytes, scrypt_derive_bytes,
    splitmix64_next_u64, zlib_compress_bytes, zlib_decompress_bytes, AesExpandedKey,
};

#[test]
fn test_int_sec01_os_csprng_non_invertible_vs_deterministic_prng() {
    // 1. Deterministic mode (`Some(&mut seed)`) must produce identical outputs across runs
    let mut seed_a = 0x1234_5678_9ABC_DEF0u64;
    let mut seed_b = 0x1234_5678_9ABC_DEF0u64;
    let uuid_det_a = generate_uuid_v4(Some(&mut seed_a));
    let uuid_det_b = generate_uuid_v4(Some(&mut seed_b));
    assert_eq!(uuid_det_a, uuid_det_b);

    // 2. Host CSPRNG mode (`None`) must produce valid RFC 4122 v4 UUIDs and must NOT follow SplitMix64 state progression
    let mut b1 = [0u8; 8];
    let mut b2 = [0u8; 8];
    fill_random_bytes(&mut b1, None);
    fill_random_bytes(&mut b2, None);
    assert_ne!(b1, [0u8; 8]);
    assert_ne!(b1, b2);

    // Invert SplitMix64 on `b1` and verify `b2` is NOT the next SplitMix64 output
    let invert_splitmix64 = |mut z: u64| -> u64 {
        z ^= (z >> 31) ^ (z >> 62);
        z = z.wrapping_mul(0x3196_42B2_D24D_8EC3);
        z ^= (z >> 27) ^ (z >> 54);
        z = z.wrapping_mul(0x96DE_1B17_3F11_9089);
        z ^= (z >> 30) ^ (z >> 60);
        z
    };
    let word1 = u64::from_le_bytes(b1);
    let word2 = u64::from_le_bytes(b2);
    let mut recovered_state = invert_splitmix64(word1);
    let predicted_next = splitmix64_next_u64(&mut recovered_state);
    assert_ne!(
        word2, predicted_next,
        "Production CSPRNG must not be predictable via SplitMix64 state inversion"
    );
}

#[test]
fn test_int_sec02_ed25519_pkcs8_strict_oid_and_pubkey_validation() {
    let raw_seed = [0x42u8; 32];
    let (seed, pub_bytes) = ed25519_keypair_from_seed(raw_seed).expect("valid seed");

    // 1. Valid raw 32-byte seed, 48-byte PKCS#8 v1, and 85-byte PKCS#8 v2 must succeed
    assert_eq!(extract_ed25519_seed(&seed).unwrap(), seed);
    let pkcs8_v1 = ed25519_seed_to_pkcs8_v1(&seed);
    assert_eq!(extract_ed25519_seed(&pkcs8_v1).unwrap(), seed);
    let pkcs8_v2 = ed25519_seed_and_pub_to_pkcs8_v2(&seed, &pub_bytes);
    assert_eq!(extract_ed25519_seed(&pkcs8_v2).unwrap(), seed);

    // 2. PKCS#8 v1 with trailing garbage bytes must be rejected
    let mut v1_trailing = pkcs8_v1.clone();
    v1_trailing.push(0x99);
    assert!(
        extract_ed25519_seed(&v1_trailing).is_err(),
        "PKCS#8 v1 with trailing bytes must be rejected"
    );

    // 3. PKCS#8 v2 with spoofed X25519 OID (`1.3.101.110` -> `0x2b, 0x65, 0x6e`) must be rejected
    let mut spoofed_oid = pkcs8_v2.clone();
    spoofed_oid[11] = 0x6e;
    assert!(
        extract_ed25519_seed(&spoofed_oid).is_err(),
        "Non-Ed25519 OID must be rejected"
    );

    // 4. PKCS#8 v2 with mismatched embedded public key must be rejected
    let mut tampered_pub = pkcs8_v2.clone();
    tampered_pub[60] ^= 0xFF;
    assert!(
        extract_ed25519_seed(&tampered_pub).is_err(),
        "PKCS#8 v2 with mismatched public key must be rejected"
    );
}

#[test]
fn test_int_sec03_kdf_and_compression_sandbox_memory_and_header_hardening() {
    // 1. Enforce sandbox memory budget on PBKDF2, HKDF, scrypt, and zlib
    BOA_KV_MEM_BUDGET.with(|m| m.set((0, 64 * 1024))); // 64 KB budget

    assert!(
        pbkdf2_derive_bytes("SHA-256", b"pw", b"salt", 1000, 128 * 1024).is_err(),
        "PBKDF2 exceeding sandbox memory ceiling must fail"
    );
    assert!(
        pbkdf2_derive_bytes("SHA-256", b"pw", b"salt", 50_000_000, 32).is_err(),
        "PBKDF2 exceeding max iterations must fail"
    );
    assert!(
        hkdf_derive_bytes("SHA-256", b"ikm", b"salt", b"info", 65 * 1024).is_err(),
        "HKDF exceeding sandbox memory ceiling must fail"
    );
    // scrypt with N=1024, r=8 requires 128 * 8 * 1024 = 1 MB working set (> 64 KB budget)
    assert!(
        scrypt_derive_bytes(b"pw", b"salt", 1024, 8, 1, 32, 32 * 1024 * 1024).is_err(),
        "scrypt working set exceeding sandbox memory ceiling must fail"
    );
    assert!(
        zlib_compress_bytes(&vec![b'a'; 128 * 1024], "gzip").is_err(),
        "zlib_compress_bytes exceeding sandbox memory ceiling must fail"
    );

    // Restore unlimited budget
    BOA_KV_MEM_BUDGET.with(|m| m.set((0, usize::MAX)));

    // 2. Malformed Gzip header with FNAME flag (`0x08`) and NO null byte before the 8-byte trailer
    //    (even if the 8-byte trailer contains 0x00 bytes!) must be rejected without panicking
    let mut bad_gzip_fname = vec![0x1F, 0x8B, 0x08, 0x08, 0, 0, 0, 0, 0, 0xFF];
    bad_gzip_fname.extend_from_slice(b"abcd"); // no NUL terminator in body
    bad_gzip_fname.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]); // NULs only inside 8-byte trailer
    assert!(
        zlib_decompress_bytes(&bad_gzip_fname, "gzip").is_err(),
        "Unterminated Gzip FNAME header must not scan into trailer"
    );

    // 3. Malformed Gzip header with FEXTRA xlen overflowing into trailer
    let bad_gzip_fextra = vec![
        0x1F, 0x8B, 0x08, 0x04, 0, 0, 0, 0, 0, 0xFF, 0x10, 0x00, // xlen = 16
        1, 2, 3, 4, 5, 6, 7, 8,
    ];
    assert!(zlib_decompress_bytes(&bad_gzip_fextra, "gzip").is_err());
}

#[test]
fn test_int_sec04_whatwg_url_authority_confusion_and_host_validation() {
    // 1. Backslash authority confusion (`http://evil.com\@127.0.0.1/secret`) in special schemes
    //    must split authority at `\` so hostname is `evil.com`, never `127.0.0.1`
    let parsed = parse_whatwg_url("http://evil.com\\@127.0.0.1/secret", None)
        .expect("valid special URL with backslash path");
    assert_eq!(parsed.hostname, "evil.com");
    assert_eq!(parsed.username, "");
    assert_eq!(parsed.pathname, "/@127.0.0.1/secret");

    // 2. Embedded ASCII tab/CR/LF must be stripped per WHATWG URL specification
    let parsed_tabs =
        parse_whatwg_url("https://ex\tample.com/a\r\nb?x=1", None).expect("tabs/newlines stripped");
    assert_eq!(parsed_tabs.hostname, "example.com");
    assert_eq!(parsed_tabs.pathname, "/ab");

    // 3. Forbidden host characters (spaces, angle brackets, unclosed IPv6) must be rejected
    assert!(parse_whatwg_url("http://exam ple.com/path", None).is_none());
    assert!(parse_whatwg_url("http://exa<mple>.com/path", None).is_none());
    assert!(parse_whatwg_url("http://[::1/path", None).is_none());
    assert!(parse_whatwg_url("http://[::1]:8080/path", None).is_some());
}

#[test]
fn test_int_perf01_02_03_aes_cbc_crc32_deflate_base64_microbenchmarks() {
    // 1. Branchless AES-256-CBC round-trip & microbenchmark
    let key = [0x3Cu8; 32];
    let iv = [0xA5u8; 16];
    let plain_1kb = vec![0x5Au8; 1024];
    let expanded = AesExpandedKey::new(&key).expect("expand AES-256 key");
    let ct = aes_cbc_encrypt_with_expanded(&expanded, &iv, &plain_1kb, true).unwrap();
    let pt = aes_cbc_decrypt_with_expanded(&expanded, &iv, &ct, true).unwrap();
    assert_eq!(pt, plain_1kb);

    let iters = 2_000u32;
    let t0 = std::time::Instant::now();
    for _ in 0..iters {
        let c = aes_cbc_encrypt_with_expanded(
            std::hint::black_box(&expanded),
            std::hint::black_box(&iv),
            std::hint::black_box(&plain_1kb),
            true,
        )
        .unwrap();
        let p = aes_cbc_decrypt_with_expanded(
            std::hint::black_box(&expanded),
            std::hint::black_box(&iv),
            std::hint::black_box(&c),
            true,
        )
        .unwrap();
        std::hint::black_box(p.len());
    }
    let aes_us_per_op = t0.elapsed().as_nanos() as f64 / (iters as f64 * 1000.0);

    // 2. Table-driven CRC-32 & 9-bit Huffman LUT Gzip round-trip & microbenchmark
    let sample_4kb =
        r#"{"service":"slice-edge","status":"ok","items":[1,2,3,4,5],"region":"us-east-1"}"#
            .repeat(52);
    let sample_bytes = sample_4kb.as_bytes();
    let crc = crc32_checksum(sample_bytes);
    assert_ne!(crc, 0);

    let t1 = std::time::Instant::now();
    for _ in 0..iters {
        std::hint::black_box(crc32_checksum(std::hint::black_box(sample_bytes)));
    }
    let crc_ns_per_op = t1.elapsed().as_nanos() as f64 / (iters as f64);

    let gz = zlib_compress_bytes(sample_bytes, "gzip").expect("gzip compress");
    let decompressed = zlib_decompress_bytes(&gz, "gzip").expect("gzip decompress");
    assert_eq!(decompressed, sample_bytes);

    let t2 = std::time::Instant::now();
    for _ in 0..iters {
        let d = zlib_decompress_bytes(std::hint::black_box(&gz), "gzip").unwrap();
        std::hint::black_box(d.len());
    }
    let gunzip_us_per_op = t2.elapsed().as_nanos() as f64 / (iters as f64 * 1000.0);

    // 3. Base64 LUT encode + decode round-trip & microbenchmark
    let b64 = base64_encode(&plain_1kb, false);
    let dec = base64_decode(&b64);
    assert_eq!(dec, plain_1kb);

    let t3 = std::time::Instant::now();
    for _ in 0..iters {
        let e = base64_encode(std::hint::black_box(&plain_1kb), false);
        let d = base64_decode(std::hint::black_box(&e));
        std::hint::black_box(d.len());
    }
    let b64_ns_per_op = t3.elapsed().as_nanos() as f64 / (iters as f64);

    eprintln!(
        "[INT-PERF] AES-256-CBC 1KB seal+open: {:.2} us/op | CRC32 4KB: {:.1} ns/op | Gunzip 4KB: {:.2} us/op | Base64 1KB enc+dec: {:.1} ns/op",
        aes_us_per_op, crc_ns_per_op, gunzip_us_per_op, b64_ns_per_op
    );
}
