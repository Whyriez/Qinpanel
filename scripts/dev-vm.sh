#!/usr/bin/env bash
# scripts/dev-vm.sh: Setup and manage Ubuntu 24.04 development VM using Multipass or Lima

set -euo pipefail

VM_NAME="nusa-dev"
VM_CPUS="2"
VM_MEM="2G"
VM_DISK="20G"
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

detect_provider() {
    if command -v multipass &>/dev/null; then
        echo "multipass"
    elif command -v limactl &>/dev/null; then
        echo "lima"
    else
        echo "none"
    fi
}

PROVIDER=$(detect_provider)

print_help() {
    echo "Usage: $0 [up|shell|test|stop|destroy|status]"
    echo ""
    echo "Commands:"
    echo "  up       Create and provision Ubuntu 24.04 VM with repo mounted"
    echo "  shell    Open interactive shell inside dev VM"
    echo "  test     Run workspace tests inside dev VM"
    echo "  stop     Stop the dev VM"
    echo "  destroy  Delete the dev VM"
    echo "  status   Check dev VM status"
    echo ""
    echo "Detected VM provider: ${PROVIDER}"
}

ensure_provider() {
    if [ "$PROVIDER" = "none" ]; then
        echo "Error: Neither 'multipass' nor 'limactl' was found."
        echo "Please install Multipass (https://multipass.run/) or Lima (https://lima-vm.io/)."
        exit 1
    fi
}

vm_up() {
    ensure_provider
    echo "==> Starting dev VM (${VM_NAME}) using ${PROVIDER}..."

    if [ "$PROVIDER" = "multipass" ]; then
        if multipass info "$VM_NAME" &>/dev/null; then
            echo "VM '${VM_NAME}' already exists. Ensuring it is running..."
            multipass start "$VM_NAME" || true
        else
            echo "Creating Multipass VM: Ubuntu 24.04 (noble)..."
            multipass launch 24.04 --name "$VM_NAME" --cpus "$VM_CPUS" --memory "$VM_MEM" --disk "$VM_DISK"
            echo "Provisioning packages inside VM..."
            multipass exec "$VM_NAME" -- sudo apt-get update
            multipass exec "$VM_NAME" -- sudo apt-get install -y \
                build-essential \
                pkg-config \
                libssl-dev \
                curl \
                git \
                nginx \
                acl \
                systemd
            echo "Installing Rust toolchain inside VM..."
            multipass exec "$VM_NAME" -- bash -c "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"
            echo "Mounting repository..."
            multipass mount "$REPO_DIR" "${VM_NAME}:/home/ubuntu/panel"
        fi
        echo "Dev VM is ready. Run '$0 shell' to enter."
    elif [ "$PROVIDER" = "lima" ]; then
        if limactl list | grep -q "$VM_NAME"; then
            limactl start "$VM_NAME" || true
        else
            limactl create --name="$VM_NAME" template://ubuntu-24.04 --cpus="$VM_CPUS" --memory="$VM_MEM" --disk="$VM_DISK"
            limactl start "$VM_NAME"
        fi
        echo "Lima VM '${VM_NAME}' is ready."
    fi
}

vm_shell() {
    ensure_provider
    if [ "$PROVIDER" = "multipass" ]; then
        multipass shell "$VM_NAME"
    elif [ "$PROVIDER" = "lima" ]; then
        limactl shell "$VM_NAME"
    fi
}

vm_test() {
    ensure_provider
    echo "==> Running tests inside ${VM_NAME}..."
    if [ "$PROVIDER" = "multipass" ]; then
        multipass exec "$VM_NAME" -- bash -c "cd /home/ubuntu/panel && source ~/.cargo/env && cargo test --workspace"
    elif [ "$PROVIDER" = "lima" ]; then
        limactl shell "$VM_NAME" bash -c "cd /home/ubuntu/panel && source ~/.cargo/env && cargo test --workspace"
    fi
}

vm_stop() {
    ensure_provider
    if [ "$PROVIDER" = "multipass" ]; then
        multipass stop "$VM_NAME"
    elif [ "$PROVIDER" = "lima" ]; then
        limactl stop "$VM_NAME"
    fi
}

vm_destroy() {
    ensure_provider
    echo "Destroying ${VM_NAME}..."
    if [ "$PROVIDER" = "multipass" ]; then
        multipass delete "$VM_NAME" || true
        multipass purge || true
    elif [ "$PROVIDER" = "lima" ]; then
        limactl delete -f "$VM_NAME" || true
    fi
}

vm_status() {
    ensure_provider
    if [ "$PROVIDER" = "multipass" ]; then
        multipass list
    elif [ "$PROVIDER" = "lima" ]; then
        limactl list
    fi
}

CMD="${1:-help}"
case "$CMD" in
    up) vm_up ;;
    shell) vm_shell ;;
    test) vm_test ;;
    stop) vm_stop ;;
    destroy) vm_destroy ;;
    status) vm_status ;;
    help|--help|-h) print_help ;;
    *) echo "Unknown command: $CMD"; print_help; exit 1 ;;
esac
