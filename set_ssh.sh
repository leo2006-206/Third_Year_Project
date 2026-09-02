#!/bin/bash

# run with 
# source set_ssh.sh

# Start the SSH agent
eval "$(ssh-agent -s)"

# Add your specific SSH key
ssh-add ~/.ssh/leowong121073/id_ed25519

# Test the connection to GitHub
ssh -T git@github.com