#!/usr/bin/env python3
"""
Package the webOS application directory into an installable .ipk (ar archive).
"""
import io
import os
import tarfile
import subprocess
from pathlib import Path

ROOT_DIR = Path(__file__).resolve().parent.parent
APP_DIR = ROOT_DIR / "webos-app"
OUTPUT_IPK = ROOT_DIR / "target" / "org.webosbrew.lg-hue-sync_0.3.0_all.ipk"

def make_tarfile_bytes(files_dict):
    """Create a tar.gz in memory from a dict of {arcname: (bytes, mode)}"""
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        for arcname, (data, mode) in files_dict.items():
            info = tarfile.TarInfo(name=arcname)
            info.size = len(data)
            info.mode = mode
            info.uid = 0
            info.gid = 0
            info.uname = "root"
            info.gname = "root"
            tar.addfile(info, io.BytesIO(data))
    return buf.getvalue()

def build_ipk():
    OUTPUT_IPK.parent.mkdir(parents=True, exist_ok=True)
    
    # 1. debian-binary
    debian_binary = b"2.0\n"
    
    # 2. control.tar.gz
    control_content = (
        "Package: org.webosbrew.lg-hue-sync\n"
        "Version: 0.3.0\n"
        "Section: misc\n"
        "Priority: optional\n"
        "Architecture: all\n"
        "Maintainer: Adam <adam@adeze.com>\n"
        "Description: Philips Hue & Nanoleaf Ambient Lighting Controller for LG C1\n"
    ).encode("utf-8")
    
    control_tar = make_tarfile_bytes({
        "./control": (control_content, 0o644)
    })
    
    # 3. data.tar.gz
    data_files = {}
    base_target = "usr/palm/applications/org.webosbrew.lg-hue-sync"
    for item in APP_DIR.rglob("*"):
        if item.is_file():
            rel = item.relative_to(APP_DIR).as_posix()
            arcname = f"./{base_target}/{rel}"
            data_files[arcname] = (item.read_bytes(), 0o755 if item.suffix == ".sh" else 0o644)
            
    data_tar = make_tarfile_bytes(data_files)
    
    # Assemble AR archive
    # Debian ar format:
    # 8-byte magic: !<arch>\n
    # Each entry: 60-byte header + data + optional padding \n if odd length
    # Header:
    # name (16 chars, slash-terminated: "debian-binary/  ")
    # mtime (12 chars: "0           ")
    # uid (6 chars: "0     ")
    # gid (6 chars: "0     ")
    # mode (8 chars: "100644  ")
    # size (10 chars: left-justified)
    # 2-byte magic: `\n
    
    def ar_entry(name, content):
        header = io.BytesIO()
        header.write(f"{name:<16}".encode("ascii"))
        header.write(f"{'0':<12}".encode("ascii"))
        header.write(f"{'0':<6}".encode("ascii"))
        header.write(f"{'0':<6}".encode("ascii"))
        header.write(f"{'100644':<8}".encode("ascii"))
        header.write(f"{len(content):<10}".encode("ascii"))
        header.write(b"`\n")
        entry = header.getvalue() + content
        if len(content) % 2 != 0:
            entry += b"\n"
        return entry

    with open(OUTPUT_IPK, "wb") as f:
        f.write(b"!<arch>\n")
        f.write(ar_entry("debian-binary", debian_binary))
        f.write(ar_entry("control.tar.gz", control_tar))
        f.write(ar_entry("data.tar.gz", data_tar))
        
    print(f"Successfully created {OUTPUT_IPK} ({os.path.getsize(OUTPUT_IPK)} bytes)")

if __name__ == "__main__":
    build_ipk()
