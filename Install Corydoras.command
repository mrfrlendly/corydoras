#!/bin/bash
# Double-click this file in Finder to build and install Corydoras.
cd "$(dirname "$0")" || exit 1
echo "Corydoras installer"
echo
read -r -p "Also add the Corydoras MCP server to Claude Desktop? A backup of its config is made first. [y/N] " ans
if [[ "$ans" =~ ^[Yy]$ ]]; then
  bash scripts/setup-mac.sh --register-claude
else
  bash scripts/setup-mac.sh
fi
status=$?
echo
if [[ $status -eq 0 ]]; then echo "Finished."; else echo "The installer stopped with an error (code $status). Copy the text above into the chat."; fi
read -n 1 -s -r -p "Press any key to close this window."
