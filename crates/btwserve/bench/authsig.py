"""Test wallets for contract.py's auth group, with nothing outside the standard library but `cryptography`
(for Ed25519): Keccak-256 and an EIP-191 personal_sign over secp256k1 (R||S||v, low S, v = 27 + recovery id),
as MetaMask and Go's auth_test.go make them. Slow and not constant-time: for tests only."""
import hashlib, hmac

# ---------- Keccak-256 (the pre-SHA-3 padding Ethereum uses) ----------

_RC = [0x0000000000000001, 0x0000000000008082, 0x800000000000808A, 0x8000000080008000, 0x000000000000808B,
       0x0000000080000001, 0x8000000080008081, 0x8000000000008009, 0x000000000000008A, 0x0000000000000088,
       0x0000000080008009, 0x000000008000000A, 0x000000008000808B, 0x800000000000008B, 0x8000000000008089,
       0x8000000000008003, 0x8000000000008002, 0x8000000000000080, 0x000000000000800A, 0x800000008000000A,
       0x8000000080008081, 0x8000000000008080, 0x0000000080000001, 0x8000000080008008]
_ROT = [[0, 36, 3, 41, 18], [1, 44, 10, 45, 2], [62, 6, 43, 15, 61], [28, 55, 25, 21, 56], [27, 20, 39, 8, 14]]
_M = (1 << 64) - 1


def _rol(x, n):
    return ((x << n) | (x >> (64 - n))) & _M if n else x


def _f(a):
    for rc in _RC:
        c = [a[x][0] ^ a[x][1] ^ a[x][2] ^ a[x][3] ^ a[x][4] for x in range(5)]
        d = [c[(x - 1) % 5] ^ _rol(c[(x + 1) % 5], 1) for x in range(5)]
        a = [[a[x][y] ^ d[x] for y in range(5)] for x in range(5)]
        b = [[0] * 5 for _ in range(5)]
        for x in range(5):
            for y in range(5):
                b[y][(2 * x + 3 * y) % 5] = _rol(a[x][y], _ROT[x][y])
        a = [[b[x][y] ^ ((~b[(x + 1) % 5][y]) & b[(x + 2) % 5][y]) for y in range(5)] for x in range(5)]
        a[0][0] ^= rc
    return a


def keccak256(data: bytes) -> bytes:
    rate = 136
    p = bytearray(data) + b"\x01"
    p += b"\x00" * ((-len(p)) % rate)
    p[-1] |= 0x80
    a = [[0] * 5 for _ in range(5)]
    for off in range(0, len(p), rate):
        blk = p[off:off + rate]
        for i in range(rate // 8):
            x, y = i % 5, i // 5
            a[x][y] ^= int.from_bytes(blk[8 * i:8 * i + 8], "little")
        a = _f(a)
    out = b""
    for i in range(4):
        out += a[i % 5][i // 5].to_bytes(8, "little")
    return out


assert keccak256(b"").hex() == "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"

# ---------- secp256k1 ----------

P = 2**256 - 2**32 - 977
N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
G = (0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798,
     0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8)


def _add(p, q):
    if p is None:
        return q
    if q is None:
        return p
    if p[0] == q[0] and (p[1] + q[1]) % P == 0:
        return None
    if p == q:
        l = 3 * p[0] * p[0] * pow(2 * p[1], -1, P) % P
    else:
        l = (q[1] - p[1]) * pow(q[0] - p[0], -1, P) % P
    x = (l * l - p[0] - q[0]) % P
    return x, (l * (p[0] - x) - p[1]) % P


def _mul(k, p=G):
    r = None
    while k:
        if k & 1:
            r = _add(r, p)
        p = _add(p, p)
        k >>= 1
    return r


class EthKey:
    def __init__(self, seed: bytes):
        self.d = int.from_bytes(hashlib.sha256(seed).digest(), "big") % (N - 1) + 1
        x, y = _mul(self.d)
        self.address = "0x" + keccak256(x.to_bytes(32, "big") + y.to_bytes(32, "big"))[12:].hex()

    def personal_sign(self, msg: str) -> str:
        m = msg.encode()
        z = int.from_bytes(keccak256(b"\x19Ethereum Signed Message:\n" + str(len(m)).encode() + m), "big")
        k = int.from_bytes(hmac.new(self.d.to_bytes(32, "big"), z.to_bytes(32, "big"), hashlib.sha256).digest(), "big") % (N - 1) + 1
        R = _mul(k)
        r = R[0] % N
        s = pow(k, -1, N) * (z + r * self.d) % N
        rec = R[1] & 1
        if s > N // 2:
            s, rec = N - s, rec ^ 1
        return "0x" + (r.to_bytes(32, "big") + s.to_bytes(32, "big") + bytes([27 + rec])).hex()


class EdKey:
    def __init__(self, seed: bytes):
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
        from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
        self.k = Ed25519PrivateKey.from_private_bytes(hashlib.sha256(seed).digest())
        self.pub = self.k.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw).hex()

    def sign(self, msg: str) -> str:
        return self.k.sign(msg.encode()).hex()


# the private key 1's address, a well-known vector
_x, _y = _mul(1)
assert keccak256(_x.to_bytes(32, "big") + _y.to_bytes(32, "big"))[12:].hex() == "7e5f4552091a69125d5dfcb7b8c2659029395bdf"
