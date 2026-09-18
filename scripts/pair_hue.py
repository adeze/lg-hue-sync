#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""
pair_hue.py
Discovers Philips Hue Bridge on the local network, guides pushlink authentication,
extracts Hue Entertainment credentials (username + clientkey for DTLS PSK),
and optionally configures Hyperion.NG on your LG C1 TV via JSON-RPC.

Usage:
    uv run scripts/pair_hue.py
    uv run scripts/pair_hue.py --tv-ip 192.168.1.149
"""

import argparse
import json
import socket
import sys
import time
import urllib.error
import urllib.request
from typing import Any, Dict, List, Optional


def discover_bridge_nupnp() -> Optional[str]:
    """Attempt discovery via official Hue discovery portal."""
    print("[*] Searching for Hue Bridge via discovery.meethue.com...")
    try:
        req = urllib.request.Request(
            "https://discovery.meethue.com/",
            headers={"User-Agent": "lg-hue-sync-pair/1.0"},
        )
        with urllib.request.urlopen(req, timeout=5) as response:
            data = json.loads(response.read().decode("utf-8"))
            if data and isinstance(data, list) and len(data) > 0:
                ip = data[0].get("internalipaddress")
                if ip:
                    print(f"[+] Found Hue Bridge via discovery API: {ip}")
                    return ip
    except Exception as e:
        print(f"[-] N-UPnP discovery failed: {e}")
    return None


def discover_bridge_ssdp(timeout: int = 3) -> Optional[str]:
    """Discover Hue Bridge on local LAN via SSDP multicast."""
    print("[*] Searching for Hue Bridge via SSDP multicast...")
    ssdp_request = (
        "M-SEARCH * HTTP/1.1\r\n"
        "HOST: 239.255.255.250:1900\r\n"
        'MAN: "ssdp:discover"\r\n'
        "MX: 2\r\n"
        "ST: ssdp:all\r\n\r\n"
    )
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
    sock.settimeout(timeout)
    try:
        sock.sendto(ssdp_request.encode("utf-8"), ("239.255.255.250", 1900))
        start_time = time.time()
        while time.time() - start_time < timeout:
            try:
                data, addr = sock.recvfrom(2048)
                decoded = data.decode("utf-8", errors="ignore")
                if "hue-bridge" in decoded.lower() or "philips" in decoded.lower():
                    print(f"[+] Found Hue Bridge via SSDP at {addr[0]}")
                    return addr[0]
            except socket.timeout:
                break
    except Exception as e:
        print(f"[-] SSDP discovery failed: {e}")
    finally:
        sock.close()
    return None


def poll_pushlink(bridge_ip: str, timeout_seconds: int = 45) -> Dict[str, str]:
    """Polls bridge for pairing button press."""
    url = f"http://{bridge_ip}/api"
    payload = json.dumps(
        {
            "devicetype": "lg-hue-sync#tv",
            "generateclientkey": True,
        }
    ).encode("utf-8")

    print("\n" + "=" * 60)
    print(">>> ACTION REQUIRED:")
    print(">>> Press the big round button on your Philips Hue Bridge now!")
    print("=" * 60 + "\n")

    start_time = time.time()
    while time.time() - start_time < timeout_seconds:
        try:
            req = urllib.request.Request(
                url,
                data=payload,
                headers={"Content-Type": "application/json"},
                method="POST",
            )
            with urllib.request.urlopen(req, timeout=3) as resp:
                result = json.loads(resp.read().decode("utf-8"))
                if isinstance(result, list) and len(result) > 0:
                    first = result[0]
                    if "success" in first:
                        success = first["success"]
                        username = success.get("username")
                        clientkey = success.get("clientkey")
                        print(f"[+] Successfully paired with Hue Bridge!")
                        print(f"[+] Username: {username}")
                        print(f"[+] Client Key (DTLS PSK): {clientkey}")
                        return {
                            "bridge_ip": bridge_ip,
                            "username": username,
                            "clientkey": clientkey,
                        }
                    elif "error" in first:
                        err_type = first["error"].get("type")
                        # error 101: link button not pressed
                        if err_type == 101:
                            sys.stdout.write(".")
                            sys.stdout.flush()
                        else:
                            print(f"\n[-] Bridge error: {first['error'].get('description')}")
        except Exception as e:
            sys.stdout.write("x")
            sys.stdout.flush()

        time.sleep(1.0)

    print("\n[-] Timed out waiting for pushlink button press.")
    sys.exit(1)


def get_entertainment_areas(bridge_ip: str, username: str) -> List[Dict[str, Any]]:
    """Fetches entertainment groups from Hue Bridge."""
    url = f"http://{bridge_ip}/api/{username}/groups"
    try:
        with urllib.request.urlopen(url, timeout=5) as resp:
            groups = json.loads(resp.read().decode("utf-8"))
            areas = []
            for gid, info in groups.items():
                if info.get("type") == "Entertainment":
                    areas.append(
                        {
                            "id": gid,
                            "name": info.get("name"),
                            "lights": info.get("lights", []),
                            "locations": info.get("locations", {}),
                        }
                    )
            return areas
    except Exception as e:
        print(f"[-] Could not fetch groups from bridge: {e}")
        return []


def configure_hyperion(
    hyperion_host: str,
    bridge_ip: str,
    username: str,
    clientkey: str,
    group_id: str,
) -> bool:
    """Configures Hyperion.NG via JSON-RPC API."""
    url = f"http://{hyperion_host}:8090/json-rpc"
    print(f"[*] Configuring Hyperion.NG at {hyperion_host}:8090...")

    # Configure Philips Hue Entertainment driver in Hyperion
    payload = {
        "command": "config",
        "subcommand": "setComponent",
        "component": "device",
        "device": {
            "type": "philipshue",
            "output": bridge_ip,
            "username": username,
            "clientkey": clientkey,
            "groupId": int(group_id),
            "useEntertainmentAPI": True,
            "sslVersion": "DTLSv1_2",
            "blacklevel": 0.0,
            "brightness": 1.0,
        },
    }

    try:
        req = urllib.request.Request(
            url,
            data=json.dumps(payload).encode("utf-8"),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=5) as resp:
            res = json.loads(resp.read().decode("utf-8"))
            if res.get("success"):
                print("[+] Hyperion.NG Philips Hue device configured successfully!")
                return True
            else:
                print(f"[-] Hyperion config returned: {res}")
    except Exception as e:
        print(f"[-] Could not communicate with Hyperion at {url}: {e}")
    return False


def main():
    parser = argparse.ArgumentParser(description="Pair with Philips Hue Bridge and configure entertainment sync.")
    parser.add_argument("--bridge-ip", help="Manual Philips Hue Bridge IP address (skips discovery)")
    parser.add_argument("--tv-ip", help="LG C1 TV IP address (to configure Hyperion.NG directly)")
    parser.add_argument("--output-config", default="config.json", help="Path to save credentials for Rust daemon")
    args = parser.parse_args()

    bridge_ip = args.bridge_ip
    if not bridge_ip:
        bridge_ip = discover_bridge_nupnp() or discover_bridge_ssdp()

    if not bridge_ip:
        bridge_ip = input("Enter Hue Bridge IP address manually: ").strip()

    creds = poll_pushlink(bridge_ip)

    # Fetch entertainment areas
    areas = get_entertainment_areas(bridge_ip, creds["username"])
    selected_area_id = "1"
    if not areas:
        print("[!] Warning: No Entertainment Areas found on this Hue Bridge.")
        print("[!] Please create an Entertainment Area in the Philips Hue mobile app.")
    else:
        print(f"\n[+] Found {len(areas)} Entertainment Area(s):")
        for i, a in enumerate(areas):
            print(f"  [{i+1}] ID: {a['id']} | Name: {a['name']} | Lights: {len(a['lights'])}")

        if len(areas) == 1:
            selected_area_id = areas[0]["id"]
            print(f"[*] Auto-selecting only area: '{areas[0]['name']}' (ID: {selected_area_id})")
        else:
            choice = input(f"Select an area [1-{len(areas)}]: ").strip()
            try:
                selected_area_id = areas[int(choice) - 1]["id"]
            except (ValueError, IndexError):
                selected_area_id = areas[0]["id"]

    creds["entertainment_area_id"] = selected_area_id

    # Save to config.json
    with open(args.output_config, "w") as f:
        json.dump(creds, f, indent=2)
    print(f"\n[+] Saved credentials to {args.output_config}")

    # Optionally push to Hyperion on TV
    if args.tv_ip:
        configure_hyperion(
            args.tv_ip,
            creds["bridge_ip"],
            creds["username"],
            creds["clientkey"],
            creds["entertainment_area_id"],
        )

    print("\nSetup complete!")


if __name__ == "__main__":
    main()
