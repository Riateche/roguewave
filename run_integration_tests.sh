#!/bin/bash

set -e

docker stop openssh-server || true
docker build -t openssh-server .github/workflows
docker run -d \
    --name=openssh-server \
    --hostname=openssh-server \
    -p 2222:22 \
    --rm \
    openssh-server

chmod 600 .github/workflows/ci_ssh_key
eval $(ssh-agent)
ssh-add .github/workflows/ci_ssh_key
ssh-keygen -R '[127.0.0.1]:2222'
for i in {1..10}; do
    sleep 0.3
    if ssh -o StrictHostKeyChecking=no -p 2222 root@127.0.0.1 "echo ssh is available"; then
        break
    fi
done
export ROGUEWAVE_INTEGRATION_TEST_DESTINATION="ssh://root@127.0.0.1:2222"
cargo test integration -- --nocapture
docker stop openssh-server
