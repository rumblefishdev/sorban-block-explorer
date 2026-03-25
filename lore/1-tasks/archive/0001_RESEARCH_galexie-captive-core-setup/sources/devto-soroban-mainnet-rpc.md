---
url: 'https://dev.to/row-bear/rpc-for-soroban-mainnet-36eo'
title: 'RPC for Soroban Mainnet'
fetched_date: 2026-03-25
task_id: '0001'
image_count: 0
---

# RPC for Soroban Mainnet

"It's here! Soroban smart contracts have launched on Stellar mainnet on 20 Feb 2024 at 17:00 UTC."

The validators approved the upgrade at 17:00 UTC, with ledger **50457424** being "the first ledger after that...running on protocol 20." Within seconds, the initial smart contract was deployed: `CDGFGODCSQQQFFBUEJGS7NIDUA5OQKBRFQ57IOKZAZIEJDNQNHS3RV5O`

## The RPC Challenge

Following launch, developer activity remained minimal initially. The author identifies two contributing factors:

1. Soroban entered phase 0 with restrictive resource limits
2. "The lack of RPC servers" emerged as a significant barrier—the Stellar Development Foundation provided public RPC access for testnet and futurenet, but not mainnet

## Solutions

Developers have two options:

- Consult the Soroban documentation's "list of public RPC providers" offering both paid and free plans
- Run a private RPC server locally

## 10-Step Setup Guide for Linux

**Requirements:**

- ~80GB disk space
- ~1 hour initial sync time
- ~10 minutes for restart syncing
- Internet connection
- Modest hardware (author uses i5-12450H CPU, 16GB RAM)

### Steps 1-5: Installation & Launch

1. Install Docker
2. Create a folder (e.g., `/home/row-bear/soroban-rpc`)
3. Execute: `docker pull stellar/soroban-rpc image`
4. Add `stellar-core.toml` configuration file to the folder
5. Run the Docker container with specific parameters

### Sample Docker Command

```plaintext
docker run -p 8001:8001 -p 8000:8000 \
--name sorobanrpc \
-v /home/row-bear/soroban-rpc:/config stellar/soroban-rpc \
--captive-core-config-path="/config/stellar-core.toml" \
--captive-core-storage-path="/var/lib/stellar/captive-core" \
--stellar-core-binary-path="/usr/bin/stellar-core" \
--db-path="/var/lib/stellar/soroban-rpc-db.sqlite" \
--stellar-captive-core-http-port=11626 \
--network-passphrase="Public Global Stellar Network ; September 2015" \
--history-archive-urls="https://history.stellar.org/prd/core-live/core_live_001" \
--admin-endpoint="0.0.0.0:8001" \
--endpoint="0.0.0.0:8000"
```

### Steps 6-10: Management & Usage

6. Find container ID via `docker container ls -a`
7. Stop/start with: `sudo docker stop <id>` and `sudo docker start <id>`
8. Monitor logs: `sudo docker logs -f sorobanrpc`
9. Check health status using cURL to test the `getHealth` method
10. Configure Soroban CLI: `soroban network add local_rpc --rpc-url http://localhost:8000 --network-passphrase "Public Global Stellar Network ; September 2015"`

## Key Technical Details

- Use the `--name` parameter when launching containers for easier management
- Employ `docker container prune` to remove stopped containers
- The configuration file determines validator trust sets
- Reference implementation: https://gist.github.com/Row-Bear/952266e78ab8a5c623786fa4fa5feba2

## Community Resources

The author recommends joining:

- Stellar Dev Discord for technical discussions
- Stellar Global Discord for general Stellar topics
