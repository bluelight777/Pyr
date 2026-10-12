#!/usr/bin/env python3
"""Let a Rust binary built on a new glibc start on older ones.

Rust's standard library references a few new glibc functions (pidfd_spawnp,
pidfd_getpid from glibc 2.39) as *weak* symbols: it checks at run time whether they
exist and falls back if not. The linker still records "needs GLIBC_2.39" as a hard
requirement, so the program refuses to start on, say, Ubuntu 22.04 (glibc 2.35).

This marks such a version requirement as weak (VER_FLG_WEAK), but only when every symbol
that needs that version is a weak reference, so a real requirement is never relaxed.

    python3 relax-glibc.py BINARY [BINARY ...]
"""
import struct
import sys

VER_FLG_WEAK = 0x2
SHT_DYNSYM, SHT_GNU_VERSYM, SHT_GNU_VERNEED = 11, 0x6FFFFFFF, 0x6FFFFFFE
STB_WEAK = 2
SHN_UNDEF = 0


def relax(path):
    data = bytearray(open(path, "rb").read())
    if data[:4] != b"\x7fELF" or data[4] != 2 or data[5] != 1:
        raise SystemExit(f"{path}: not a 64-bit little-endian ELF file")
    e_shoff, = struct.unpack_from("<Q", data, 0x28)
    e_shentsize, e_shnum = struct.unpack_from("<HH", data, 0x3A)
    sections = []
    for i in range(e_shnum):
        off = e_shoff + i * e_shentsize
        name, typ, flags, addr, offset, size, link, info, align, entsize = struct.unpack_from("<IIQQQQIIQQ", data, off)
        sections.append(dict(type=typ, offset=offset, size=size, link=link, entsize=entsize))

    def find(t):
        return next((s for s in sections if s["type"] == t), None)

    dynsym, versym, verneed = find(SHT_DYNSYM), find(SHT_GNU_VERSYM), find(SHT_GNU_VERNEED)
    if not (dynsym and versym and verneed):
        print(f"{path}: no symbol versioning, nothing to do")
        return
    strtab = sections[verneed["link"]]

    def cstr(off):
        end = data.index(b"\0", strtab["offset"] + off)
        return data[strtab["offset"] + off:end].decode()

    # Which version indexes are used by strong references (or by defined symbols)?
    strong = set()
    count = dynsym["size"] // 24
    for i in range(1, count):
        st_name, st_info, st_other, st_shndx = struct.unpack_from("<IBBH", data, dynsym["offset"] + i * 24)
        idx, = struct.unpack_from("<H", data, versym["offset"] + i * 2)
        idx &= 0x7FFF
        if st_info >> 4 != STB_WEAK or st_shndx != SHN_UNDEF:
            strong.add(idx)

    changed = []
    off = verneed["offset"]
    while True:
        vn_version, vn_cnt, vn_file, vn_aux, vn_next = struct.unpack_from("<HHIII", data, off)
        lib = cstr(vn_file)
        aux = off + vn_aux
        for _ in range(vn_cnt):
            vna_hash, vna_flags, vna_other, vna_name, vna_next = struct.unpack_from("<IHHII", data, aux)
            name = cstr(vna_name)
            if lib.startswith("libc.so") and vna_other not in strong and not vna_flags & VER_FLG_WEAK:
                struct.pack_into("<H", data, aux + 4, vna_flags | VER_FLG_WEAK)
                changed.append(name)
            if not vna_next:
                break
            aux += vna_next
        if not vn_next:
            break
        off += vn_next
    if changed:
        open(path, "wb").write(data)
        print(f"{path}: marked weak (only weak symbols need them): {', '.join(changed)}")
    else:
        print(f"{path}: nothing to relax")


if __name__ == "__main__":
    for p in sys.argv[1:]:
        relax(p)
