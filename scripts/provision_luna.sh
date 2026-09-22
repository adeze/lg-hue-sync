#!/usr/bin/env bash
set -euo pipefail

# provision_luna.sh
# Provisions Luna Service 2 security manifests, roles, and client permissions
# for org.webosbrew.lg-hue-sync on LG webOS 6.x (LG C1 OLED).

TV_IP="${1:-192.168.1.149}"
SSH_PORT="${2:-22}"

SSH_OPTS="-p $SSH_PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=5"

echo "[*] Provisioning Luna permissions for org.webosbrew.lg-hue-sync on $TV_IP:$SSH_PORT..."

provision_files() {
    mkdir -p /var/luna-service2/manifests.d /var/luna-service2/roles.d /var/luna-service2/client-permissions.d
    mkdir -p /var/luna-service2-dev/manifests.d /var/luna-service2-dev/roles.d /var/luna-service2-dev/client-permissions.d

    # 1. Manifests
    cat << 'JSON' > /var/luna-service2/manifests.d/org.webosbrew.lg-hue-sync.json
{
    "id": "org.webosbrew.lg-hue-sync",
    "version": "1.0.0",
    "roleFiles": [
        "/var/luna-service2/roles.d/org.webosbrew.lg-hue-sync.json"
    ],
    "clientPermissionFiles": [
        "/var/luna-service2/client-permissions.d/org.webosbrew.lg-hue-sync.json"
    ],
    "serviceFiles": [],
    "apiPermissionFiles": []
}
JSON

    cat << 'JSON' > /var/luna-service2-dev/manifests.d/org.webosbrew.lg-hue-sync.json
{
    "id": "org.webosbrew.lg-hue-sync",
    "version": "1.0.0",
    "roleFiles": [
        "/var/luna-service2-dev/roles.d/org.webosbrew.lg-hue-sync.json"
    ],
    "clientPermissionFiles": [
        "/var/luna-service2-dev/client-permissions.d/org.webosbrew.lg-hue-sync.json"
    ],
    "serviceFiles": [],
    "apiPermissionFiles": []
}
JSON

    # 2. Privileged Role Files (without appId)
    cat << 'JSON' > /tmp/org.webosbrew.lg-hue-sync.role.json
{
    "exeName": "/var/home/root/lg-hue-sync/lg-hue-sync",
    "type": "privileged",
    "allowedNames": [
        "com.webos.service.capture",
        "com.webos.service.capture.client*",
        "com.webos.rm.client.*",
        "com.webos.service.capturepermission",
        "org.webosbrew.lg-hue-sync*"
    ],
    "permissions": [
        {
            "service": "com.webos.service.capture",
            "inbound": ["*"],
            "outbound": ["*"]
        },
        {
            "service": "com.webos.service.capture.client*",
            "inbound": ["*"],
            "outbound": ["*"]
        },
        {
            "service": "com.webos.rm.client.*",
            "inbound": ["*"],
            "outbound": ["*"]
        },
        {
            "service": "com.webos.service.capturepermission",
            "inbound": ["*"],
            "outbound": ["*"]
        },
        {
            "service": "org.webosbrew.lg-hue-sync*",
            "inbound": ["*"],
            "outbound": ["*"]
        }
    ]
}
JSON
    cp -f /tmp/org.webosbrew.lg-hue-sync.role.json /var/luna-service2/roles.d/org.webosbrew.lg-hue-sync.json
    cp -f /tmp/org.webosbrew.lg-hue-sync.role.json /var/luna-service2-dev/roles.d/org.webosbrew.lg-hue-sync.json
    rm -f /tmp/org.webosbrew.lg-hue-sync.role.json

    # 3. Client Permissions
    cat << 'JSON' > /tmp/org.webosbrew.lg-hue-sync.perm.json
{
    "org.webosbrew.lg-hue-sync*": [
        "all"
    ],
    "com.webos.service.capture.client*": [
        "all"
    ],
    "com.webos.service.capture": [
        "all"
    ],
    "com.webos.service.capturepermission": [
        "all"
    ],
    "com.webos.rm.client.*": [
        "all"
    ]
}
JSON
    cp -f /tmp/org.webosbrew.lg-hue-sync.perm.json /var/luna-service2/client-permissions.d/org.webosbrew.lg-hue-sync.json
    cp -f /tmp/org.webosbrew.lg-hue-sync.perm.json /var/luna-service2-dev/client-permissions.d/org.webosbrew.lg-hue-sync.json
    rm -f /tmp/org.webosbrew.lg-hue-sync.perm.json

    ls-control scan-services
    echo "[+] Luna permissions provisioned and scanned successfully."
}

if [ "$TV_IP" = "local" ] || [ "$TV_IP" = "localhost" ] || [ "$TV_IP" = "127.0.0.1" ]; then
    echo "[*] Provisioning Luna permissions locally..."
    provision_files
else
    echo "[*] Provisioning Luna permissions for org.webosbrew.lg-hue-sync on $TV_IP:$SSH_PORT..."
    ssh $SSH_OPTS root@"$TV_IP" bash << 'EOF'
set -e
mkdir -p /var/luna-service2/manifests.d /var/luna-service2/roles.d /var/luna-service2/client-permissions.d
mkdir -p /var/luna-service2-dev/manifests.d /var/luna-service2-dev/roles.d /var/luna-service2-dev/client-permissions.d

cat << 'JSON' > /var/luna-service2/manifests.d/org.webosbrew.lg-hue-sync.json
{
    "id": "org.webosbrew.lg-hue-sync",
    "version": "1.0.0",
    "roleFiles": [
        "/var/luna-service2/roles.d/org.webosbrew.lg-hue-sync.json"
    ],
    "clientPermissionFiles": [
        "/var/luna-service2/client-permissions.d/org.webosbrew.lg-hue-sync.json"
    ],
    "serviceFiles": [],
    "apiPermissionFiles": []
}
JSON

cat << 'JSON' > /var/luna-service2-dev/manifests.d/org.webosbrew.lg-hue-sync.json
{
    "id": "org.webosbrew.lg-hue-sync",
    "version": "1.0.0",
    "roleFiles": [
        "/var/luna-service2-dev/roles.d/org.webosbrew.lg-hue-sync.json"
    ],
    "clientPermissionFiles": [
        "/var/luna-service2-dev/client-permissions.d/org.webosbrew.lg-hue-sync.json"
    ],
    "serviceFiles": [],
    "apiPermissionFiles": []
}
JSON

cat << 'JSON' > /tmp/org.webosbrew.lg-hue-sync.role.json
{
    "exeName": "/var/home/root/lg-hue-sync/lg-hue-sync",
    "type": "privileged",
    "allowedNames": [
        "com.webos.service.capture",
        "com.webos.service.capture.client*",
        "com.webos.rm.client.*",
        "com.webos.service.capturepermission",
        "org.webosbrew.lg-hue-sync*"
    ],
    "permissions": [
        {
            "service": "com.webos.service.capture",
            "inbound": ["*"],
            "outbound": ["*"]
        },
        {
            "service": "com.webos.service.capture.client*",
            "inbound": ["*"],
            "outbound": ["*"]
        },
        {
            "service": "com.webos.rm.client.*",
            "inbound": ["*"],
            "outbound": ["*"]
        },
        {
            "service": "com.webos.service.capturepermission",
            "inbound": ["*"],
            "outbound": ["*"]
        },
        {
            "service": "org.webosbrew.lg-hue-sync*",
            "inbound": ["*"],
            "outbound": ["*"]
        }
    ]
}
JSON
cp -f /tmp/org.webosbrew.lg-hue-sync.role.json /var/luna-service2/roles.d/org.webosbrew.lg-hue-sync.json
cp -f /tmp/org.webosbrew.lg-hue-sync.role.json /var/luna-service2-dev/roles.d/org.webosbrew.lg-hue-sync.json
rm -f /tmp/org.webosbrew.lg-hue-sync.role.json

cat << 'JSON' > /tmp/org.webosbrew.lg-hue-sync.perm.json
{
    "org.webosbrew.lg-hue-sync*": [
        "all"
    ],
    "com.webos.service.capture.client*": [
        "all"
    ],
    "com.webos.service.capture": [
        "all"
    ],
    "com.webos.service.capturepermission": [
        "all"
    ],
    "com.webos.rm.client.*": [
        "all"
    ]
}
JSON
cp -f /tmp/org.webosbrew.lg-hue-sync.perm.json /var/luna-service2/client-permissions.d/org.webosbrew.lg-hue-sync.json
cp -f /tmp/org.webosbrew.lg-hue-sync.perm.json /var/luna-service2-dev/client-permissions.d/org.webosbrew.lg-hue-sync.json
rm -f /tmp/org.webosbrew.lg-hue-sync.perm.json

ls-control scan-services
echo "[+] Luna permissions provisioned and scanned successfully."
EOF
fi
