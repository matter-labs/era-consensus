#!/bin/bash
set -euo pipefail

# Script to set up Ansible virtual environment with all dependencies

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
VENV_DIR="${SCRIPT_DIR}/ansible-venv"

echo "Setting up Ansible virtual environment..."

# Create virtual environment
python3 -m venv "${VENV_DIR}"

# Activate virtual environment
source "${VENV_DIR}/bin/activate"

# Upgrade pip
echo "Upgrading pip..."
python -m pip install --upgrade pip

# Install Python dependencies
echo "Installing Python dependencies..."
pip install -r "${SCRIPT_DIR}/requirements.txt"

# Install Ansible Galaxy requirements
echo "Installing Ansible Galaxy requirements..."
ansible-galaxy install -r "${SCRIPT_DIR}/requirements.yml"

echo "Virtual environment setup complete!"
echo ""
echo "To activate the virtual environment, run:"
echo "    source ${VENV_DIR}/bin/activate"
echo ""
echo "To deactivate, run:"
echo "    deactivate"
