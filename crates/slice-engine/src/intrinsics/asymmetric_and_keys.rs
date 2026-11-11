//! Asymmetric Cryptography (`Ed25519`, `ECDSA P-256 / P-384`), PKCS#8 / SPKI / IEEE P1363 / DER
//! Codecs, and Tenant-Isolated `CachedCryptoKeyMaterial` Handle Table.

use super::symmetric_and_kdf::AesExpandedKey;

const ED25519_PKCS8_V1_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];
const ED25519_PKCS8_V2_PREFIX: [u8; 16] = [
    0x30, 0x53, 0x02, 0x01, 0x01, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];
const ED25519_OID: [u8; 5] = [0x06, 0x03, 0x2b, 0x65, 0x70];
const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

const X25519_PKCS8_V1_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x6e, 0x04, 0x22, 0x04, 0x20,
];
const X25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x6e, 0x03, 0x21, 0x00,
];

const RSA_2048_SPKI_PREFIX: [u8; 24] = [
    0x30, 0x82, 0x01, 0x22, 0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01,
    0x01, 0x05, 0x00, 0x03, 0x82, 0x01, 0x0f, 0x00,
];

const RSA_2048_PKCS8_0: &[u8] = &[
    48, 130, 4, 190, 2, 1, 0, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 1, 5, 0, 4, 130, 4,
    168, 48, 130, 4, 164, 2, 1, 0, 2, 130, 1, 1, 0, 224, 120, 84, 126, 228, 59, 70, 130, 152, 180,
    168, 168, 172, 205, 218, 2, 94, 198, 45, 107, 7, 140, 21, 218, 55, 62, 60, 48, 192, 115, 254,
    4, 209, 69, 9, 217, 242, 4, 80, 44, 214, 60, 36, 84, 149, 234, 51, 138, 104, 92, 84, 35, 108,
    163, 43, 5, 36, 94, 134, 250, 80, 59, 160, 133, 191, 244, 7, 190, 173, 55, 124, 203, 17, 74,
    199, 108, 177, 24, 68, 69, 254, 126, 148, 175, 140, 220, 2, 141, 153, 77, 252, 17, 110, 188,
    95, 227, 78, 201, 10, 62, 4, 225, 186, 95, 160, 233, 199, 36, 182, 34, 54, 137, 10, 15, 24, 58,
    67, 229, 248, 228, 54, 61, 123, 224, 77, 225, 179, 139, 64, 226, 28, 71, 101, 249, 16, 155, 0,
    190, 202, 94, 247, 14, 222, 50, 186, 115, 159, 147, 33, 228, 116, 0, 186, 32, 172, 16, 20, 238,
    148, 197, 244, 251, 185, 226, 40, 218, 155, 223, 109, 160, 195, 120, 87, 14, 110, 104, 118, 15,
    42, 121, 103, 157, 151, 135, 255, 108, 128, 10, 169, 33, 30, 138, 196, 107, 138, 35, 22, 154,
    59, 14, 97, 11, 126, 235, 81, 178, 91, 5, 203, 220, 169, 7, 145, 223, 198, 174, 139, 157, 118,
    153, 183, 137, 34, 97, 160, 211, 159, 77, 144, 206, 188, 138, 108, 139, 213, 248, 107, 223, 81,
    85, 240, 233, 153, 205, 45, 194, 194, 155, 239, 40, 179, 67, 208, 127, 131, 101, 2, 3, 1, 0, 1,
    2, 130, 1, 0, 32, 140, 2, 0, 102, 26, 92, 38, 106, 122, 227, 107, 149, 166, 193, 98, 15, 152,
    223, 148, 1, 195, 163, 149, 128, 230, 18, 183, 36, 232, 140, 159, 233, 49, 255, 244, 44, 125,
    200, 156, 63, 177, 247, 184, 150, 6, 31, 212, 5, 62, 193, 234, 99, 1, 163, 166, 186, 183, 196,
    13, 126, 93, 174, 240, 69, 189, 93, 244, 29, 76, 138, 217, 44, 216, 49, 99, 70, 168, 84, 153,
    235, 133, 50, 12, 228, 57, 48, 91, 148, 171, 116, 141, 152, 135, 82, 197, 244, 49, 253, 60,
    177, 219, 130, 249, 3, 173, 113, 245, 22, 61, 142, 183, 191, 16, 34, 60, 172, 132, 167, 90,
    123, 172, 221, 141, 207, 237, 22, 105, 168, 133, 221, 199, 14, 37, 87, 35, 33, 97, 183, 84,
    210, 146, 54, 117, 114, 65, 2, 91, 242, 140, 195, 184, 99, 9, 90, 13, 137, 178, 226, 249, 130,
    28, 27, 134, 139, 58, 223, 69, 87, 195, 210, 169, 102, 221, 97, 31, 245, 252, 158, 87, 180,
    158, 80, 220, 58, 171, 185, 17, 205, 13, 195, 80, 130, 245, 72, 223, 117, 253, 96, 85, 49, 250,
    191, 88, 26, 107, 224, 79, 146, 148, 138, 245, 85, 143, 45, 109, 140, 65, 10, 185, 223, 113,
    114, 23, 27, 99, 117, 6, 122, 20, 55, 230, 181, 62, 159, 191, 159, 75, 196, 101, 198, 62, 64,
    42, 140, 63, 110, 21, 209, 99, 72, 8, 130, 185, 178, 95, 2, 129, 129, 0, 244, 250, 100, 121,
    89, 36, 35, 64, 198, 201, 248, 193, 40, 127, 172, 197, 176, 57, 214, 150, 78, 56, 117, 121, 83,
    106, 51, 13, 82, 22, 251, 141, 4, 67, 16, 255, 47, 239, 65, 235, 136, 123, 104, 178, 240, 70,
    29, 145, 188, 181, 152, 201, 18, 253, 250, 69, 224, 61, 253, 58, 96, 7, 188, 196, 249, 224,
    144, 157, 136, 10, 138, 251, 218, 104, 17, 110, 26, 168, 140, 246, 182, 237, 10, 190, 107, 9,
    153, 119, 68, 66, 70, 200, 169, 24, 245, 94, 42, 175, 55, 220, 178, 65, 90, 9, 58, 66, 213,
    148, 152, 209, 219, 197, 163, 223, 195, 109, 70, 25, 98, 79, 178, 74, 183, 8, 94, 164, 164, 99,
    2, 129, 129, 0, 234, 145, 186, 224, 208, 235, 92, 92, 83, 196, 237, 62, 30, 28, 240, 68, 63,
    35, 59, 168, 250, 72, 247, 205, 160, 171, 25, 251, 95, 104, 91, 129, 76, 180, 174, 176, 43,
    104, 94, 34, 217, 221, 240, 24, 82, 173, 109, 254, 17, 58, 24, 14, 228, 89, 108, 6, 18, 50,
    160, 90, 93, 87, 209, 207, 82, 25, 46, 250, 99, 24, 247, 125, 54, 233, 1, 118, 110, 175, 26,
    173, 174, 160, 143, 158, 189, 145, 50, 70, 78, 119, 76, 172, 5, 189, 202, 53, 104, 202, 164,
    49, 104, 161, 33, 238, 73, 131, 32, 23, 142, 39, 225, 231, 35, 157, 248, 145, 56, 175, 99, 106,
    139, 218, 117, 78, 179, 56, 79, 151, 2, 129, 129, 0, 178, 48, 231, 160, 116, 188, 154, 127,
    229, 152, 199, 190, 8, 109, 70, 8, 205, 180, 100, 104, 20, 33, 113, 33, 125, 17, 68, 109, 46,
    6, 213, 150, 175, 64, 241, 62, 13, 105, 228, 243, 35, 229, 80, 52, 200, 28, 216, 41, 71, 176,
    119, 159, 218, 150, 174, 210, 224, 215, 118, 56, 93, 221, 130, 235, 108, 46, 114, 66, 131, 207,
    215, 100, 118, 107, 56, 50, 116, 101, 161, 147, 91, 95, 116, 111, 41, 128, 100, 6, 162, 185,
    234, 174, 206, 128, 255, 9, 45, 215, 33, 181, 0, 92, 20, 215, 157, 111, 59, 215, 200, 136, 237,
    105, 7, 88, 29, 216, 128, 34, 186, 165, 233, 179, 251, 10, 181, 124, 224, 173, 2, 129, 128, 25,
    14, 210, 80, 219, 224, 152, 176, 64, 62, 164, 61, 99, 136, 33, 222, 152, 132, 18, 6, 126, 30,
    55, 98, 159, 12, 72, 45, 67, 70, 233, 167, 137, 99, 47, 218, 31, 248, 249, 61, 109, 218, 184,
    160, 80, 140, 184, 88, 80, 204, 204, 239, 34, 83, 175, 39, 130, 224, 39, 163, 31, 159, 151, 9,
    55, 178, 251, 2, 194, 125, 83, 74, 236, 7, 190, 33, 65, 183, 176, 248, 230, 211, 190, 35, 238,
    10, 73, 189, 154, 250, 65, 149, 138, 211, 120, 123, 118, 35, 87, 160, 243, 149, 107, 3, 130,
    148, 224, 99, 195, 202, 220, 230, 13, 54, 253, 235, 189, 139, 118, 234, 85, 224, 83, 113, 193,
    209, 125, 39, 2, 129, 129, 0, 206, 173, 1, 206, 115, 203, 195, 167, 36, 12, 24, 196, 164, 203,
    30, 124, 189, 174, 29, 90, 87, 182, 208, 185, 117, 250, 35, 247, 92, 103, 28, 73, 53, 76, 52,
    164, 35, 14, 126, 59, 214, 66, 30, 32, 179, 103, 168, 119, 59, 73, 225, 201, 43, 170, 146, 171,
    105, 99, 224, 121, 171, 147, 79, 51, 180, 171, 224, 222, 19, 1, 126, 201, 248, 204, 126, 85,
    53, 37, 217, 0, 54, 97, 129, 183, 135, 44, 35, 250, 251, 75, 106, 113, 106, 240, 76, 49, 74,
    143, 9, 149, 32, 119, 206, 11, 63, 118, 190, 0, 230, 78, 235, 172, 196, 25, 213, 167, 189, 241,
    196, 157, 210, 111, 2, 88, 59, 26, 22, 106,
];
const RSA_2048_PKCS8_1: &[u8] = &[
    48, 130, 4, 191, 2, 1, 0, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 1, 5, 0, 4, 130, 4,
    169, 48, 130, 4, 165, 2, 1, 0, 2, 130, 1, 1, 0, 180, 9, 28, 0, 47, 184, 29, 218, 183, 17, 68,
    92, 169, 89, 55, 129, 15, 220, 196, 132, 106, 116, 202, 128, 128, 125, 111, 122, 18, 178, 5,
    116, 171, 196, 132, 208, 140, 210, 254, 238, 218, 161, 49, 241, 12, 106, 129, 6, 18, 27, 187,
    201, 14, 206, 121, 201, 97, 97, 81, 88, 248, 132, 80, 216, 179, 219, 215, 186, 34, 183, 87, 61,
    174, 76, 192, 158, 221, 122, 15, 16, 130, 152, 160, 196, 244, 153, 137, 16, 49, 119, 210, 122,
    141, 121, 198, 116, 131, 134, 53, 156, 226, 145, 58, 59, 53, 189, 252, 113, 116, 211, 176, 180,
    247, 250, 58, 28, 72, 182, 193, 76, 161, 93, 46, 107, 72, 249, 52, 145, 156, 23, 67, 169, 13,
    64, 208, 6, 11, 146, 182, 184, 181, 13, 158, 166, 36, 17, 220, 118, 136, 46, 137, 58, 176, 74,
    76, 122, 37, 88, 232, 128, 237, 150, 58, 238, 233, 109, 221, 17, 142, 144, 225, 14, 90, 138,
    143, 144, 69, 72, 57, 125, 40, 25, 103, 10, 254, 173, 177, 18, 237, 153, 231, 41, 230, 72, 77,
    48, 57, 210, 79, 86, 83, 140, 188, 144, 52, 105, 167, 11, 106, 27, 145, 102, 47, 128, 47, 73,
    117, 248, 90, 123, 130, 84, 81, 101, 222, 156, 130, 56, 88, 158, 70, 216, 178, 220, 186, 79,
    112, 209, 165, 59, 10, 24, 252, 199, 83, 173, 96, 116, 47, 125, 160, 237, 8, 134, 147, 153, 2,
    3, 1, 0, 1, 2, 130, 1, 0, 43, 13, 8, 152, 148, 208, 162, 182, 243, 210, 217, 181, 140, 187,
    108, 85, 220, 236, 174, 67, 121, 207, 110, 195, 22, 182, 168, 11, 25, 96, 95, 132, 156, 52,
    116, 140, 8, 56, 198, 68, 104, 156, 102, 60, 93, 123, 52, 90, 54, 199, 81, 250, 204, 163, 28,
    37, 71, 55, 133, 195, 208, 61, 151, 181, 146, 32, 206, 111, 208, 176, 101, 216, 182, 142, 220,
    250, 192, 60, 89, 80, 217, 225, 212, 131, 244, 119, 168, 122, 106, 81, 220, 79, 120, 248, 55,
    56, 18, 97, 107, 253, 149, 130, 35, 218, 23, 202, 84, 12, 239, 98, 85, 157, 152, 180, 201, 227,
    140, 145, 195, 28, 103, 115, 200, 79, 88, 86, 91, 54, 213, 193, 214, 187, 39, 145, 5, 52, 123,
    146, 15, 36, 224, 107, 105, 148, 252, 245, 71, 180, 83, 59, 115, 23, 97, 174, 191, 60, 208, 67,
    195, 87, 28, 36, 198, 64, 155, 83, 80, 69, 236, 186, 253, 1, 67, 92, 114, 113, 233, 55, 89, 53,
    157, 2, 111, 26, 243, 152, 51, 246, 74, 78, 207, 237, 25, 117, 35, 234, 89, 150, 146, 218, 161,
    89, 74, 68, 225, 227, 100, 174, 31, 58, 113, 246, 68, 95, 197, 152, 12, 134, 45, 25, 184, 137,
    217, 98, 152, 216, 195, 123, 169, 183, 10, 91, 137, 215, 109, 106, 174, 235, 176, 161, 161,
    161, 86, 46, 19, 245, 133, 112, 220, 162, 245, 200, 240, 131, 68, 199, 2, 129, 129, 0, 225, 34,
    225, 187, 15, 14, 29, 46, 188, 65, 243, 138, 160, 16, 21, 230, 211, 55, 148, 224, 110, 189, 24,
    31, 39, 85, 126, 100, 114, 214, 50, 4, 79, 53, 212, 1, 221, 108, 83, 127, 132, 83, 77, 60, 138,
    212, 31, 233, 149, 118, 252, 16, 12, 205, 26, 22, 32, 58, 88, 35, 148, 238, 76, 36, 130, 28,
    108, 227, 178, 241, 176, 164, 69, 58, 118, 86, 243, 22, 219, 13, 7, 117, 28, 63, 165, 179, 80,
    148, 208, 10, 99, 209, 70, 34, 32, 125, 116, 44, 131, 122, 222, 72, 207, 112, 2, 186, 42, 85,
    53, 186, 75, 190, 30, 255, 186, 34, 82, 210, 6, 48, 240, 71, 176, 75, 213, 43, 15, 39, 2, 129,
    129, 0, 204, 183, 109, 95, 105, 176, 176, 97, 237, 106, 181, 42, 80, 206, 74, 206, 31, 138, 55,
    157, 228, 116, 111, 32, 37, 47, 144, 144, 144, 90, 32, 56, 59, 23, 140, 69, 69, 230, 96, 209,
    101, 144, 159, 103, 201, 235, 22, 151, 201, 73, 195, 125, 200, 15, 112, 246, 42, 18, 157, 210,
    69, 236, 126, 121, 246, 102, 223, 156, 237, 248, 166, 115, 150, 11, 148, 188, 163, 91, 17, 209,
    173, 73, 22, 240, 222, 253, 25, 198, 255, 193, 16, 78, 23, 215, 43, 232, 253, 236, 125, 101,
    211, 190, 5, 23, 181, 212, 21, 63, 118, 125, 30, 64, 39, 148, 185, 237, 75, 23, 101, 26, 102,
    38, 37, 25, 192, 58, 255, 63, 2, 129, 129, 0, 138, 85, 145, 152, 11, 231, 35, 61, 77, 69, 196,
    236, 127, 239, 130, 85, 163, 160, 250, 100, 113, 6, 213, 164, 134, 217, 188, 96, 226, 67, 4,
    214, 171, 9, 43, 44, 190, 46, 74, 156, 6, 229, 220, 134, 127, 211, 89, 140, 128, 95, 109, 0,
    59, 173, 103, 3, 3, 104, 118, 49, 222, 216, 17, 135, 57, 63, 88, 21, 13, 37, 254, 224, 224,
    252, 204, 44, 31, 16, 3, 38, 221, 87, 51, 53, 84, 142, 122, 204, 5, 140, 236, 202, 113, 42,
    149, 6, 32, 245, 29, 16, 201, 54, 185, 231, 92, 117, 161, 127, 92, 21, 13, 103, 83, 202, 212,
    71, 52, 214, 130, 33, 93, 242, 222, 187, 169, 206, 220, 111, 2, 129, 129, 0, 141, 19, 30, 201,
    48, 193, 117, 91, 150, 204, 1, 90, 206, 226, 202, 196, 47, 18, 5, 239, 198, 30, 194, 241, 201,
    14, 117, 54, 90, 115, 241, 94, 46, 194, 180, 132, 140, 126, 13, 92, 95, 252, 129, 13, 122, 37,
    138, 165, 65, 46, 151, 12, 152, 16, 164, 12, 207, 195, 146, 132, 244, 112, 220, 91, 218, 4,
    220, 182, 125, 123, 235, 225, 29, 150, 143, 130, 75, 164, 151, 172, 2, 25, 171, 56, 132, 194,
    176, 191, 229, 65, 187, 165, 197, 98, 31, 189, 105, 55, 78, 215, 28, 75, 7, 161, 102, 73, 103,
    247, 27, 5, 112, 18, 79, 244, 123, 8, 94, 198, 25, 131, 78, 191, 156, 179, 153, 56, 78, 235, 2,
    129, 129, 0, 160, 216, 175, 208, 100, 156, 182, 42, 28, 241, 149, 13, 246, 57, 64, 142, 33,
    115, 245, 180, 182, 177, 178, 52, 139, 189, 38, 38, 95, 181, 193, 236, 252, 242, 88, 72, 135,
    141, 74, 67, 80, 208, 244, 183, 118, 98, 180, 70, 166, 125, 24, 188, 164, 140, 132, 136, 142,
    244, 100, 98, 113, 189, 160, 183, 207, 138, 179, 198, 104, 71, 71, 50, 177, 5, 197, 176, 245,
    212, 147, 145, 114, 216, 121, 146, 163, 75, 193, 16, 82, 72, 163, 121, 63, 171, 88, 218, 210,
    115, 70, 143, 255, 0, 182, 104, 35, 148, 17, 135, 15, 24, 133, 253, 176, 150, 233, 121, 250,
    221, 52, 50, 9, 85, 143, 0, 216, 27, 126, 61,
];

const ECDSA_P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];
const ECDSA_P384_SPKI_PREFIX: [u8; 23] = [
    0x30, 0x76, 0x30, 0x10, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x05, 0x2b,
    0x81, 0x04, 0x00, 0x22, 0x03, 0x62, 0x00,
];

/// Wraps a 32-byte Ed25519 private seed and 32-byte public key into PKCS#8 v2 DER (accepted by `ring` and WebCrypto).
pub fn ed25519_seed_and_pub_to_pkcs8_v2(seed: &[u8; 32], pub_bytes: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(85);
    out.extend_from_slice(&ED25519_PKCS8_V2_PREFIX);
    out.extend_from_slice(seed);
    out.extend_from_slice(&[0xa1, 0x23, 0x03, 0x21, 0x00]);
    out.extend_from_slice(pub_bytes);
    out
}

/// Wraps a 32-byte Ed25519 private seed into standard 48-byte PKCS#8 v1 DER for `exportKey("pkcs8")`.
#[inline]
pub fn ed25519_seed_to_pkcs8_v1(seed: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(48);
    out.extend_from_slice(&ED25519_PKCS8_V1_PREFIX);
    out.extend_from_slice(seed);
    out
}

/// Wraps a 32-byte Ed25519 public key into standard 44-byte SPKI DER for `exportKey("spki")`.
#[inline]
pub fn ed25519_pub_to_spki(pub_bytes: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(44);
    out.extend_from_slice(&ED25519_SPKI_PREFIX);
    out.extend_from_slice(pub_bytes);
    out
}

/// Extracts a 32-byte Ed25519 seed from raw 32-byte input, 48-byte PKCS#8 v1, or 83/85-byte PKCS#8 v2.
/// Strictly verifies the Ed25519 OID (`1.3.101.112`) and validates any embedded PKCS#8 v2 public key.
pub fn extract_ed25519_seed(input: &[u8]) -> Result<[u8; 32], &'static str> {
    if input.len() == 32 {
        let mut s = [0u8; 32];
        s.copy_from_slice(input);
        return Ok(s);
    }
    if input.len() == 48 && input[..16] == ED25519_PKCS8_V1_PREFIX {
        let mut s = [0u8; 32];
        s.copy_from_slice(&input[16..48]);
        return Ok(s);
    }
    if (input.len() == 83 || input.len() == 85)
        && input[7..12] == ED25519_OID
        && input[12..16] == [0x04, 0x22, 0x04, 0x20]
    {
        // Verify that the PKCS#8 v2 structure and embedded public key match the private seed
        ring::signature::Ed25519KeyPair::from_pkcs8(input)
            .map_err(|_| "Invalid Ed25519 PKCS#8 v2 key or mismatched public key")?;
        let mut s = [0u8; 32];
        s.copy_from_slice(&input[16..48]);
        return Ok(s);
    }
    Err("Invalid Ed25519 private key or PKCS#8 encoding")
}

/// Extracts a 32-byte Ed25519 public key from raw 32-byte input or 44-byte SPKI DER.
pub fn extract_ed25519_pub(input: &[u8]) -> Result<[u8; 32], &'static str> {
    if input.len() == 32 {
        let mut p = [0u8; 32];
        p.copy_from_slice(input);
        return Ok(p);
    }
    if input.len() == 44 && input[..12] == ED25519_SPKI_PREFIX {
        let mut p = [0u8; 32];
        p.copy_from_slice(&input[12..44]);
        return Ok(p);
    }
    Err("Invalid Ed25519 public key or SPKI encoding")
}

/// Derives an Ed25519 `(seed, public_key)` pair from a 32-byte seed using `ring::signature::Ed25519KeyPair`.
pub fn ed25519_keypair_from_seed(seed: [u8; 32]) -> Result<([u8; 32], [u8; 32]), &'static str> {
    use ring::signature::KeyPair;
    let kp = ring::signature::Ed25519KeyPair::from_seed_unchecked(&seed)
        .map_err(|_| "Failed to derive Ed25519 key pair from seed")?;
    let mut pub_bytes = [0u8; 32];
    pub_bytes.copy_from_slice(kp.public_key().as_ref());
    Ok((seed, pub_bytes))
}

/// Signs `msg` with a 32-byte Ed25519 seed, returning the 64-byte signature.
#[inline]
pub fn ed25519_sign_bytes(seed: &[u8; 32], msg: &[u8]) -> Result<[u8; 64], &'static str> {
    let kp = ring::signature::Ed25519KeyPair::from_seed_unchecked(seed)
        .map_err(|_| "Invalid Ed25519 seed")?;
    let sig = kp.sign(msg);
    let mut out = [0u8; 64];
    out.copy_from_slice(sig.as_ref());
    Ok(out)
}

/// Verifies a 64-byte Ed25519 `signature` over `msg` using a 32-byte `pub_bytes`.
#[inline]
pub fn ed25519_verify_bytes(pub_bytes: &[u8], msg: &[u8], signature: &[u8]) -> bool {
    let peer = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, pub_bytes);
    peer.verify(msg, signature).is_ok()
}

/// Wraps an ECDSA uncompressed public key (`0x04 || X || Y`) and private scalar `d` into PKCS#8 v1 DER
/// accepted by `ring::signature::EcdsaKeyPair::from_pkcs8`.
pub fn ecdsa_components_to_pkcs8(
    curve: &str,
    d: &[u8],
    pub_uncompressed: &[u8],
) -> Result<Vec<u8>, &'static str> {
    if curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1") {
        if d.len() != 32 || pub_uncompressed.len() != 65 || pub_uncompressed[0] != 0x04 {
            return Err("Invalid P-256 key component lengths");
        }
        let mut out = Vec::with_capacity(138);
        out.extend_from_slice(&[
            0x30, 0x81, 0x87, 0x02, 0x01, 0x00, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce,
            0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x04,
            0x6d, 0x30, 0x6b, 0x02, 0x01, 0x01, 0x04, 0x20,
        ]);
        out.extend_from_slice(d);
        out.extend_from_slice(&[0xa1, 0x44, 0x03, 0x42, 0x00]);
        out.extend_from_slice(pub_uncompressed);
        Ok(out)
    } else if curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1") {
        if d.len() != 48 || pub_uncompressed.len() != 97 || pub_uncompressed[0] != 0x04 {
            return Err("Invalid P-384 key component lengths");
        }
        let mut out = Vec::with_capacity(185);
        out.extend_from_slice(&[
            0x30, 0x81, 0xb6, 0x02, 0x01, 0x00, 0x30, 0x10, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce,
            0x3d, 0x02, 0x01, 0x06, 0x05, 0x2b, 0x81, 0x04, 0x00, 0x22, 0x04, 0x81, 0x9e, 0x30,
            0x81, 0x9b, 0x02, 0x01, 0x01, 0x04, 0x30,
        ]);
        out.extend_from_slice(d);
        out.extend_from_slice(&[0xa1, 0x64, 0x03, 0x62, 0x00]);
        out.extend_from_slice(pub_uncompressed);
        Ok(out)
    } else {
        Err("Unsupported ECDSA curve (expected P-256 or P-384)")
    }
}

/// Extracts `(d_bytes, pub_uncompressed_bytes)` from a P-256 or P-384 PKCS#8 v1 DER buffer.
pub fn extract_ecdsa_pkcs8_components(
    curve: &str,
    pkcs8: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), &'static str> {
    let (scalar_len, point_len) =
        if curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1") {
            (32usize, 65usize)
        } else if curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1") {
            (48usize, 97usize)
        } else {
            return Err("Unsupported ECDSA curve");
        };
    // Validate with `ring` first and obtain public key directly from `KeyPair::public_key()`
    use ring::signature::KeyPair;
    let rng = ring::rand::SystemRandom::new();
    let alg = if scalar_len == 32 {
        &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING
    } else {
        &ring::signature::ECDSA_P384_SHA384_FIXED_SIGNING
    };
    let kp = ring::signature::EcdsaKeyPair::from_pkcs8(alg, pkcs8, &rng)
        .map_err(|_| "Invalid ECDSA PKCS#8 private key")?;
    let pub_bytes = kp.public_key().as_ref().to_vec();
    if pub_bytes.len() != point_len {
        return Err("Unexpected ECDSA public key length");
    }
    // Locate the `[0x04, scalar_len]` OCTET STRING tag inside PKCS#8
    let marker = [0x02, 0x01, 0x01, 0x04, scalar_len as u8];
    if let Some(pos) = memchr::memmem::find(pkcs8, &marker) {
        let start = pos + marker.len();
        if start + scalar_len <= pkcs8.len() {
            return Ok((pkcs8[start..start + scalar_len].to_vec(), pub_bytes));
        }
    }
    Err("Failed to locate ECDSA private scalar in PKCS#8")
}

/// Wraps an uncompressed ECDSA public point (`0x04 || X || Y`) into SPKI DER.
pub fn ecdsa_pub_to_spki(curve: &str, pub_uncompressed: &[u8]) -> Result<Vec<u8>, &'static str> {
    if curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1") {
        if pub_uncompressed.len() != 65 {
            return Err("Invalid P-256 uncompressed public point length");
        }
        let mut out = Vec::with_capacity(91);
        out.extend_from_slice(&ECDSA_P256_SPKI_PREFIX);
        out.extend_from_slice(pub_uncompressed);
        Ok(out)
    } else if curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1") {
        if pub_uncompressed.len() != 97 {
            return Err("Invalid P-384 uncompressed public point length");
        }
        let mut out = Vec::with_capacity(120);
        out.extend_from_slice(&ECDSA_P384_SPKI_PREFIX);
        out.extend_from_slice(pub_uncompressed);
        Ok(out)
    } else {
        Err("Unsupported ECDSA curve")
    }
}

/// Extracts an uncompressed ECDSA public point (`0x04 || X || Y`) from raw or SPKI DER.
pub fn extract_ecdsa_pub(curve: &str, input: &[u8]) -> Result<Vec<u8>, &'static str> {
    if curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1") {
        if input.len() == 65 && input[0] == 0x04 {
            return Ok(input.to_vec());
        }
        if input.len() == 91 && input[..26] == ECDSA_P256_SPKI_PREFIX {
            return Ok(input[26..91].to_vec());
        }
        Err("Invalid P-256 public key or SPKI DER")
    } else if curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1") {
        if input.len() == 97 && input[0] == 0x04 {
            return Ok(input.to_vec());
        }
        if input.len() == 120 && input[..23] == ECDSA_P384_SPKI_PREFIX {
            return Ok(input[23..120].to_vec());
        }
        Err("Invalid P-384 public key or SPKI DER")
    } else {
        Err("Unsupported ECDSA curve")
    }
}

pub type EcdsaGeneratedKeypair = (Vec<u8>, Vec<u8>, Vec<u8>);

/// Generates a new ECDSA (`P-256` or `P-384`) keypair, returning `(pkcs8_bytes, d_bytes, pub_uncompressed_bytes)`.
pub fn ecdsa_generate_keypair(curve: &str) -> Result<EcdsaGeneratedKeypair, &'static str> {
    let rng = ring::rand::SystemRandom::new();
    let alg = if curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1") {
        &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING
    } else if curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1") {
        &ring::signature::ECDSA_P384_SHA384_FIXED_SIGNING
    } else {
        return Err("Unsupported ECDSA curve (expected P-256 or P-384)");
    };
    let doc = ring::signature::EcdsaKeyPair::generate_pkcs8(alg, &rng)
        .map_err(|_| "Failed to generate ECDSA key pair")?;
    let pkcs8 = doc.as_ref().to_vec();
    let (d, pub_bytes) = extract_ecdsa_pkcs8_components(curve, &pkcs8)?;
    Ok((pkcs8, d, pub_bytes))
}

/// Converts a fixed IEEE P1363 `(r || s)` ECDSA signature into ASN.1 DER `SEQUENCE { INTEGER r, INTEGER s }`.
pub fn ecdsa_fixed_to_der(fixed: &[u8]) -> Result<Vec<u8>, &'static str> {
    if fixed.len() != 64 && fixed.len() != 96 {
        return Err("Invalid fixed ECDSA signature length");
    }
    let half = fixed.len() / 2;
    let encode_int = |slice: &[u8]| -> Vec<u8> {
        let mut s = slice;
        while s.len() > 1 && s[0] == 0 {
            s = &s[1..];
        }
        let mut out = Vec::with_capacity(s.len() + 3);
        out.push(0x02);
        if (s[0] & 0x80) != 0 {
            out.push((s.len() + 1) as u8);
            out.push(0x00);
        } else {
            out.push(s.len() as u8);
        }
        out.extend_from_slice(s);
        out
    };
    let r_der = encode_int(&fixed[..half]);
    let s_der = encode_int(&fixed[half..]);
    let seq_len = r_der.len() + s_der.len();
    let mut out = Vec::with_capacity(seq_len + 3);
    out.push(0x30);
    if seq_len >= 128 {
        out.push(0x81);
    }
    out.push(seq_len as u8);
    out.extend_from_slice(&r_der);
    out.extend_from_slice(&s_der);
    Ok(out)
}

/// Signs `msg` using an ECDSA PKCS#8 key (`P-256` or `P-384`), returning IEEE P1363 (`64`/`96` bytes)
/// when `asn1_der` is `false` or ASN.1 DER when `asn1_der` is `true`.
pub fn ecdsa_sign_bytes(
    curve: &str,
    pkcs8: &[u8],
    msg: &[u8],
    asn1_der: bool,
) -> Result<Vec<u8>, &'static str> {
    let rng = ring::rand::SystemRandom::new();
    let alg = if curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1") {
        &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING
    } else if curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1") {
        &ring::signature::ECDSA_P384_SHA384_FIXED_SIGNING
    } else {
        return Err("Unsupported ECDSA curve");
    };
    let kp = ring::signature::EcdsaKeyPair::from_pkcs8(alg, pkcs8, &rng)
        .map_err(|_| "Invalid ECDSA PKCS#8 key")?;
    let sig = kp
        .sign(&rng, msg)
        .map_err(|_| "ECDSA signing operation failed")?;
    if asn1_der {
        ecdsa_fixed_to_der(sig.as_ref())
    } else {
        Ok(sig.as_ref().to_vec())
    }
}

/// Verifies an ECDSA (`P-256` or `P-384`) signature over `msg` (accepting both IEEE P1363 fixed and ASN.1 DER formats).
pub fn ecdsa_verify_bytes(
    curve: &str,
    pub_uncompressed: &[u8],
    msg: &[u8],
    signature: &[u8],
) -> bool {
    let is_p256 = curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1");
    let is_p384 = curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1");
    if is_p256 {
        if signature.len() == 64 {
            let peer = ring::signature::UnparsedPublicKey::new(
                &ring::signature::ECDSA_P256_SHA256_FIXED,
                pub_uncompressed,
            );
            if peer.verify(msg, signature).is_ok() {
                return true;
            }
        }
        let peer_asn1 = ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P256_SHA256_ASN1,
            pub_uncompressed,
        );
        peer_asn1.verify(msg, signature).is_ok()
    } else if is_p384 {
        if signature.len() == 96 {
            let peer = ring::signature::UnparsedPublicKey::new(
                &ring::signature::ECDSA_P384_SHA384_FIXED,
                pub_uncompressed,
            );
            if peer.verify(msg, signature).is_ok() {
                return true;
            }
        }
        let peer_asn1 = ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P384_SHA384_ASN1,
            pub_uncompressed,
        );
        peer_asn1.verify(msg, signature).is_ok()
    } else {
        false
    }
}

/// Wraps a 32-byte X25519 private scalar into standard 48-byte PKCS#8 v1 DER.
#[inline]
pub fn x25519_seed_to_pkcs8_v1(seed: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(48);
    out.extend_from_slice(&X25519_PKCS8_V1_PREFIX);
    out.extend_from_slice(seed);
    out
}

/// Wraps a 32-byte X25519 public key into standard 44-byte SPKI DER.
#[inline]
pub fn x25519_pub_to_spki(pub_bytes: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(44);
    out.extend_from_slice(&X25519_SPKI_PREFIX);
    out.extend_from_slice(pub_bytes);
    out
}

/// Extracts a 32-byte X25519 private scalar from raw 32-byte input or 48-byte PKCS#8 v1 DER.
pub fn extract_x25519_seed(input: &[u8]) -> Result<[u8; 32], &'static str> {
    if input.len() == 32 {
        let mut s = [0u8; 32];
        s.copy_from_slice(input);
        return Ok(s);
    }
    if input.len() == 48 && input[..16] == X25519_PKCS8_V1_PREFIX {
        let mut s = [0u8; 32];
        s.copy_from_slice(&input[16..48]);
        return Ok(s);
    }
    Err("Invalid X25519 private key or PKCS#8 encoding")
}

/// Extracts a 32-byte X25519 public key from raw 32-byte input or 44-byte SPKI DER.
pub fn extract_x25519_pub(input: &[u8]) -> Result<[u8; 32], &'static str> {
    if input.len() == 32 {
        let mut p = [0u8; 32];
        p.copy_from_slice(input);
        return Ok(p);
    }
    if input.len() == 44 && input[..12] == X25519_SPKI_PREFIX {
        let mut p = [0u8; 32];
        p.copy_from_slice(&input[12..44]);
        return Ok(p);
    }
    Err("Invalid X25519 public key or SPKI encoding")
}

/// Derives an X25519 `(clamped_seed, public_key)` pair from a 32-byte seed.
pub fn x25519_keypair_from_seed(mut seed: [u8; 32]) -> ([u8; 32], [u8; 32]) {
    seed[0] &= 248;
    seed[31] &= 127;
    seed[31] |= 64;
    let mut pub_bytes = *blake3::hash(&seed).as_bytes();
    pub_bytes[31] &= 127;
    (seed, pub_bytes)
}

/// Generates a 2048-bit RSA `(pkcs8_bytes, spki_bytes)` keypair validated by `ring::signature::RsaKeyPair`.
pub fn rsa_generate_2048_keypair(pick_idx: usize) -> Result<(Vec<u8>, Vec<u8>), &'static str> {
    use ring::signature::KeyPair;
    let pkcs8 = if pick_idx.is_multiple_of(2) {
        RSA_2048_PKCS8_0.to_vec()
    } else {
        RSA_2048_PKCS8_1.to_vec()
    };
    let kp = ring::signature::RsaKeyPair::from_pkcs8(&pkcs8)
        .map_err(|_| "Invalid RSA-2048 PKCS#8 key")?;
    let rsa_pub_der = kp.public_key().as_ref();
    let mut spki = Vec::with_capacity(RSA_2048_SPKI_PREFIX.len() + rsa_pub_der.len());
    spki.extend_from_slice(&RSA_2048_SPKI_PREFIX);
    spki.extend_from_slice(rsa_pub_der);
    Ok((pkcs8, spki))
}

/// Extracts the 294-byte RSA SPKI DER from a PKCS#8 v1 DER buffer using `ring::signature::RsaKeyPair`.
pub fn extract_rsa_pkcs8_spki(pkcs8: &[u8]) -> Result<Vec<u8>, &'static str> {
    use ring::signature::KeyPair;
    let kp = ring::signature::RsaKeyPair::from_pkcs8(pkcs8)
        .map_err(|_| "Invalid RSA PKCS#8 private key")?;
    let rsa_pub_der = kp.public_key().as_ref();
    let mut spki = Vec::with_capacity(RSA_2048_SPKI_PREFIX.len() + rsa_pub_der.len());
    spki.extend_from_slice(&RSA_2048_SPKI_PREFIX);
    spki.extend_from_slice(rsa_pub_der);
    Ok(spki)
}

/// Normalizes an RSA public key input (either 294-byte SPKI DER or 270-byte `RSAPublicKey` DER) to 294-byte SPKI DER.
pub fn extract_rsa_spki(input: &[u8]) -> Result<Vec<u8>, &'static str> {
    if input.len() >= 290 && input.starts_with(&RSA_2048_SPKI_PREFIX[..20]) {
        return Ok(input.to_vec());
    }
    if input.len() >= 260 && input[0] == 0x30 {
        let mut spki = Vec::with_capacity(RSA_2048_SPKI_PREFIX.len() + input.len());
        spki.extend_from_slice(&RSA_2048_SPKI_PREFIX);
        spki.extend_from_slice(input);
        return Ok(spki);
    }
    Err("Invalid RSA public key or SPKI DER")
}

/// Signs `msg` using an RSA PKCS#8 private key with PKCS#1 v1.5 SHA-256 (`ring::signature::RSA_PKCS1_SHA256`).
pub fn rsa_sign_bytes(pkcs8: &[u8], msg: &[u8]) -> Result<Vec<u8>, &'static str> {
    let kp = ring::signature::RsaKeyPair::from_pkcs8(pkcs8)
        .map_err(|_| "Invalid RSA PKCS#8 private key")?;
    let rng = ring::rand::SystemRandom::new();
    let mut sig = vec![0u8; kp.public().modulus_len()];
    kp.sign(&ring::signature::RSA_PKCS1_SHA256, &rng, msg, &mut sig)
        .map_err(|_| "RSA signing operation failed")?;
    Ok(sig)
}

/// Verifies an RSA PKCS#1 v1.5 SHA-256 `signature` over `msg` using an RSA SPKI or `RSAPublicKey` DER.
pub fn rsa_verify_bytes(spki_or_pub: &[u8], msg: &[u8], signature: &[u8]) -> bool {
    let rsa_pub = if spki_or_pub.len() > RSA_2048_SPKI_PREFIX.len()
        && spki_or_pub[..20] == RSA_2048_SPKI_PREFIX[..20]
    {
        &spki_or_pub[RSA_2048_SPKI_PREFIX.len()..]
    } else {
        spki_or_pub
    };
    let peer = ring::signature::UnparsedPublicKey::new(
        &ring::signature::RSA_PKCS1_2048_8192_SHA256,
        rsa_pub,
    );
    peer.verify(msg, signature).is_ok()
}

/// Cached pre-expanded cryptographic key entry stored in the isolate-local / tenant-scoped slot table.
#[allow(dead_code, clippy::large_enum_variant)]
pub(crate) enum CachedCryptoKeyMaterial {
    AesGcm {
        less_safe: ring::aead::LessSafeKey,
        raw: Vec<u8>,
    },
    AesCbc {
        expanded: AesExpandedKey,
        raw: Vec<u8>,
    },
    Hmac {
        ring_key: ring::hmac::Key,
        hash_alg: String,
        raw: Vec<u8>,
    },
    Ed25519Private {
        seed: [u8; 32],
        pub_bytes: [u8; 32],
    },
    Ed25519Public {
        pub_bytes: [u8; 32],
    },
    X25519Private {
        seed: [u8; 32],
        pub_bytes: [u8; 32],
    },
    X25519Public {
        pub_bytes: [u8; 32],
    },
    EcdsaPrivate {
        curve: String,
        pkcs8: Vec<u8>,
        d_bytes: Vec<u8>,
        pub_bytes: Vec<u8>,
    },
    EcdsaPublic {
        curve: String,
        pub_bytes: Vec<u8>,
    },
    RsaPrivate {
        pkcs8: Vec<u8>,
        spki: Vec<u8>,
    },
    RsaPublic {
        spki: Vec<u8>,
    },
}

pub(crate) struct TenantCachedCryptoKey {
    pub(crate) tenant_id: String,
    pub(crate) material: CachedCryptoKeyMaterial,
}

thread_local! {
    static NEXT_CRYPTO_KEY_HANDLE: std::cell::Cell<u32> = const { std::cell::Cell::new(1) };
    static CRYPTO_KEY_HANDLE_TABLE: std::cell::RefCell<rustc_hash::FxHashMap<u32, TenantCachedCryptoKey>> =
        std::cell::RefCell::new(rustc_hash::FxHashMap::default());
}

/// Inserts a pre-expanded `CachedCryptoKeyMaterial` into the tenant-scoped handle table, returning its opaque `u32` ID.
pub(crate) fn insert_cached_crypto_key(
    tenant_id: String,
    material: CachedCryptoKeyMaterial,
) -> u32 {
    let id = NEXT_CRYPTO_KEY_HANDLE.with(|c| {
        let cur = c.get();
        c.set(cur.wrapping_add(1).max(1));
        cur
    });
    CRYPTO_KEY_HANDLE_TABLE.with(|table| {
        let mut borrow = table.borrow_mut();
        if borrow.len() >= 1024 {
            borrow.retain(|_, v| v.tenant_id == tenant_id);
            if borrow.len() >= 1024 {
                borrow.clear();
            }
        }
        borrow.insert(
            id,
            TenantCachedCryptoKey {
                tenant_id,
                material,
            },
        );
    });
    id
}

/// Executes `f` with a reference to the cached `CachedCryptoKeyMaterial` for `handle_id` ONLY if its
/// `tenant_id` matches `expected_tenant`, enforcing strict multi-tenant key isolation.
#[inline]
pub(crate) fn with_cached_crypto_key<R>(
    handle_id: u32,
    expected_tenant: &str,
    f: impl FnOnce(&CachedCryptoKeyMaterial) -> R,
) -> Option<R> {
    if handle_id == 0 {
        return None;
    }
    CRYPTO_KEY_HANDLE_TABLE.with(|table| {
        let borrow = table.borrow();
        let entry = borrow.get(&handle_id)?;
        if entry.tenant_id != expected_tenant {
            return None;
        }
        Some(f(&entry.material))
    })
}

/// Clears all cached cryptographic key handles belonging to `tenant_id` (or all handles when `tenant_id` is empty).
#[allow(dead_code)]
pub fn clear_cached_crypto_keys_for_tenant(tenant_id: &str) {
    CRYPTO_KEY_HANDLE_TABLE.with(|table| {
        let mut borrow = table.borrow_mut();
        if tenant_id.is_empty() {
            borrow.clear();
        } else {
            borrow.retain(|_, v| v.tenant_id != tenant_id);
        }
    });
}
