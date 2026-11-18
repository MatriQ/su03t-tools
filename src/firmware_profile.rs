//! Shared image authentication for audio and strictly whitelisted configuration edits.
//! Restore allowed log digits to their observed baseline before checking the
//! original immutable digest. All other non-audio bytes remain authenticated.
use sha2::{Digest, Sha256};
pub(crate) struct Profile {
    pub(crate) name: &'static str,
    pub(crate) size: usize,
    pub(crate) base: usize,
    pub(crate) directory: usize,
    pub(crate) count: usize,
    pub(crate) full: bool,
    pub(crate) digest: &'static str,
    pub(crate) runtime_log: (usize, u8),
    pub(crate) engine_log: usize,
}
const PROFILES: &[Profile] = &[
    // Hashes cover the entire input with the blob and directory length fields
    // zeroed. Original firmware is not shipped in the repository.
    Profile {
        name: "official-release",
        size: 1349024,
        base: 0x407d8,
        directory: 0x68f18,
        count: 39,
        full: false,
        digest: "98861a2fc06e5d3416aa2743c8dcd558024114a3607b71c25e54322478f195f5",
        runtime_log: (0x13a749, b'0'),
        engine_log: 0x139ce3,
    },
    Profile {
        name: "official-release",
        size: 1414560,
        base: 0x507d8,
        directory: 0x78f18,
        count: 39,
        full: true,
        digest: "049b624725542d3d1a6e6c40d704eeddc989f0fd065e565faa926cfde82a4c37",
        runtime_log: (0x14a749, b'0'),
        engine_log: 0x149ce3,
    },
    Profile {
        name: "official-debug",
        size: 1349024,
        base: 0x407d8,
        directory: 0x68f18,
        count: 39,
        full: false,
        digest: "780a421f2a878b69780b9dc7c0e1643840d3350ed07a2ac4485f24c6455d3518",
        runtime_log: (0x13a749, b'1'),
        engine_log: 0x139ce3,
    },
    Profile {
        name: "official-debug",
        size: 1414560,
        base: 0x507d8,
        directory: 0x78f18,
        count: 39,
        full: true,
        digest: "fd3b9beaf00893326cd1f52263dcc1c5a931cc926374be2f3948987d636f59b7",
        runtime_log: (0x14a749, b'1'),
        engine_log: 0x149ce3,
    },
    Profile {
        name: "platform-a",
        size: 1235304,
        base: 0x42560,
        directory: 0x4ce60,
        count: 20,
        full: false,
        digest: "c7e8e21a655705d311b096eb839b0f180904444f053006bf9499637c1685adfe",
        runtime_log: (0x11de39, b'0'),
        engine_log: 0x11d3cf,
    },
    Profile {
        name: "platform-a",
        size: 1300840,
        base: 0x52560,
        directory: 0x5ce60,
        count: 20,
        full: true,
        digest: "649f5aded11182b1abc25f695f63710104559f9562673ec5f93dd81276316eb7",
        runtime_log: (0x12de39, b'0'),
        engine_log: 0x12d3cf,
    },
    Profile {
        name: "platform-b",
        size: 1259048,
        base: 0x42388,
        directory: 0x52588,
        count: 26,
        full: false,
        digest: "1e0dbbbbc424b21038bfecae48b99efebe2be78444663b7f4062f5be2b9a4a73",
        runtime_log: (0x123b01, b'0'),
        engine_log: 0x12308f,
    },
    Profile {
        name: "platform-b",
        size: 1324584,
        base: 0x52388,
        directory: 0x62588,
        count: 26,
        full: true,
        digest: "473da45d7446914852315ba8d3914ee31a95c4d55bbeb879de5012f8fadfa02b",
        runtime_log: (0x133b01, b'0'),
        engine_log: 0x13308f,
    },
    Profile {
        name: "latest-factory",
        size: 1364928,
        base: 0x42bb8,
        directory: 0x6bdb8,
        count: 36,
        full: false,
        digest: "b4ba52e70f37e5c9d677fd57913896ed25a21dbebd6ac7c0a9264add6f2004e5",
        runtime_log: (0x13d741, b'0'),
        engine_log: 0x13ccdb,
    },
    Profile {
        name: "latest-factory",
        size: 1430464,
        base: 0x52bb8,
        directory: 0x7bdb8,
        count: 36,
        full: true,
        digest: "20dc9b84e44da5313d5f2ad1d08d7f5640f4df00379965826675239cc0171090",
        runtime_log: (0x14d741, b'0'),
        engine_log: 0x14ccdb,
    },
    Profile {
        name: "latest-wake-switch",
        size: 1332896,
        base: 0x4b00c,
        directory: 0x61dcc,
        count: 25,
        full: false,
        digest: "8e272917bc718f3c972133f4634a942022c67ee281eeb9a43e56506b27359af0",
        runtime_log: (0x1335a1, b'0'),
        engine_log: 0x132a13,
    },
    Profile {
        name: "latest-wake-switch",
        size: 1398432,
        base: 0x5b00c,
        directory: 0x71dcc,
        count: 25,
        full: true,
        digest: "9dcd9508db1ee1cc4488d189be929a3d7b4a896698c0d79f3373e346ac5af781",
        runtime_log: (0x1435a1, b'0'),
        engine_log: 0x142a13,
    },
];

pub(crate) fn normalized_digest(bytes: &[u8], profile: &Profile) -> Option<String> {
    if bytes.len() != profile.size {
        return None;
    }
    let (offset, original) = profile.runtime_log;
    if !matches!(bytes.get(offset), Some(b'0' | b'1')) {
        return None;
    }
    let mut normalized = bytes.to_vec();
    normalized[offset] = original;
    normalized[profile.base..profile.directory].fill(0);
    for index in 0..profile.count {
        let offset = profile.directory + index * 12 + 8;
        normalized[offset..offset + 4].fill(0);
    }
    Some(format!("{:x}", Sha256::digest(&normalized)))
}

pub(crate) fn authenticate(bytes: &[u8]) -> Option<&'static Profile> {
    PROFILES
        .iter()
        .filter(|profile| profile.size == bytes.len())
        .find(|profile| normalized_digest(bytes, profile).as_deref() == Some(profile.digest))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_runtime_boolean_and_audio_ranges_are_normalized() {
        let profile = Profile {
            name: "synthetic",
            size: 512,
            base: 64,
            directory: 320,
            count: 1,
            full: false,
            digest: "",
            runtime_log: (400, b'0'),
            engine_log: 420,
        };
        let mut bytes = vec![0x55; 512];
        bytes[400] = b'0';
        bytes[420] = b'6';
        let original = normalized_digest(&bytes, &profile).unwrap();
        bytes[400] = b'1';
        assert_eq!(normalized_digest(&bytes, &profile).unwrap(), original);
        bytes[80] ^= 1;
        bytes[328] ^= 1;
        assert_eq!(normalized_digest(&bytes, &profile).unwrap(), original);
        bytes[420] = b'2';
        assert_ne!(normalized_digest(&bytes, &profile).unwrap(), original);
        bytes[420] = b'6';
        bytes[10] ^= 1;
        assert_ne!(normalized_digest(&bytes, &profile).unwrap(), original);
        for value in [b'2', b'9', b'x', 0, 255] {
            bytes[400] = value;
            assert!(normalized_digest(&bytes, &profile).is_none());
        }
        assert!(normalized_digest(&bytes[..511], &profile).is_none());
    }
}
