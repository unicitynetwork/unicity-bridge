# syntax=docker/dockerfile:1.7

ARG SP1_VERSION=v6.3.1
ARG SP1_IMAGE_DIGEST=sha256:0942a27dbe8e38f4b14f3732e779df4027b17bde93e9fbc9e8c773c15eb63400
ARG CARGO_PROVE_SHA256=c9d6ee7667fa9e0a2302324a6bb0295c55f6acf0e17a242ad11ee45767bb08df

# ---- guest ELF from source --------------------------------------------------
FROM ghcr.io/succinctlabs/sp1:${SP1_VERSION}@${SP1_IMAGE_DIGEST} AS guest-build
ARG SP1_VERSION
ARG CARGO_PROVE_SHA256
ADD --checksum=sha256:${CARGO_PROVE_SHA256} https://github.com/succinctlabs/sp1/releases/download/${SP1_VERSION}/cargo_prove_${SP1_VERSION}_linux_amd64.tar.gz /tmp/cargo-prove.tar.gz
RUN mkdir -p /root/.sp1/bin && tar -xzf /tmp/cargo-prove.tar.gz -C /root/.sp1/bin && chmod +x /root/.sp1/bin/cargo-prove
ENV PATH=/root/.sp1/bin:$PATH
WORKDIR /src/prover
COPY prover/ /src/prover/
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/root/.cargo/git \
    cargo prove build -p bridge-return-guest --binaries bridge-return-sp1-guest --features sp1 --locked \
      --output-directory /out --elf-name bridge-return-sp1-guest \
 && cd /out && sha256sum bridge-return-sp1-guest > bridge-return-sp1-guest.sha256 && ls -la /out

FROM scratch AS guest-from-source
COPY --from=guest-build /out/ /

# ---- the source builds the pinned guest ELF ---------------------------------
FROM guest-build AS guest-check
RUN cd /src/prover/guest-elf && sha256sum -c bridge-return-sp1-guest.sha256 \
 && cmp /out/bridge-return-sp1-guest bridge-return-sp1-guest \
 && echo "the source builds the pinned guest ELF"

# ---- host + service --------------------------------------------------------
FROM rust:1-bookworm AS host
RUN apt-get update && apt-get install -y --no-install-recommends \
      protobuf-compiler libprotobuf-dev clang pkg-config libssl-dev ca-certificates \
 && rm -rf /var/lib/apt/lists/*
ADD https://go.dev/dl/go1.24.0.linux-amd64.tar.gz /tmp/go.tar.gz
RUN tar -C /usr/local -xzf /tmp/go.tar.gz && rm /tmp/go.tar.gz
ENV PATH=/usr/local/go/bin:/root/go/bin:$PATH
ENV GOPROXY=https://proxy.golang.org,https://goproxy.cn,direct
WORKDIR /src/prover
COPY prover/ /src/prover/
COPY protocol/ /src/protocol/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/root/go/pkg \
    --mount=type=cache,target=/src/prover/target \
    cargo build --release -p bridge-return-host -p bridge-return-service \
      --features bridge-return-host/sp1 \
 && mkdir -p /out/bin && cp target/release/bridge-return-host target/release/bridge-return-service /out/bin/
ARG GUEST_DEPLOYMENT=sepolia/sepolia-usdc.json
COPY deployments/ /src/deployments/
RUN cd /src/prover/guest-elf && sha256sum -c bridge-return-sp1-guest.sha256 \
 && mkdir -p /out/sp1 && cp bridge-return-sp1-guest /out/sp1/bridge-return-sp1-guest
RUN /out/bin/bridge-return-host sp1-vkey /out/sp1/bridge-return-sp1-guest | tee /out/sp1/vkey.json \
 && node_vkey=$(sed -n 's/.*"vkey": *"\([^"]*\)".*/\1/p' /out/sp1/vkey.json) && echo "$node_vkey" > /out/sp1/vkey.txt \
 && { grep -q "\"vkey\": \"$node_vkey\"" "/src/deployments/$GUEST_DEPLOYMENT" \
      || { echo "guest verifying key $node_vkey is not the one $GUEST_DEPLOYMENT pins"; exit 1; }; } \
 && /out/bin/bridge-return-host check-vectors /src/protocol/vectors

# ---- vault artifact for the relayer ---------------------------------------
FROM node:22-bookworm AS contracts
WORKDIR /src/contracts/tron
COPY contracts/tron/package.json contracts/tron/package-lock.json ./
RUN npm ci
COPY contracts/tron/hardhat.config.js ./
COPY contracts/tron/contracts/ ./contracts/
RUN npx hardhat compile
RUN npm prune --omit=dev

# ---- vkey only ---------------------------------------------------------------
FROM scratch AS vkey
COPY --from=host /out/sp1/ /

# ---- runtime ---------------------------------------------------------------
FROM node:22-bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=host /out/bin/ /app/bin/
COPY --from=host /out/sp1/ /app/sp1/
COPY --from=contracts /src/contracts/tron/node_modules/ /app/contracts/tron/node_modules/
COPY --from=contracts /src/contracts/tron/artifacts/contracts/UnicityBridgeVault.sol/ /app/contracts/tron/artifacts/contracts/UnicityBridgeVault.sol/
COPY contracts/tron/scripts/ /app/contracts/tron/scripts/
COPY deployments/ /app/deployments/
COPY bft-trustbase.testnet2.json /app/
COPY prover/docker/entrypoint.sh /app/entrypoint.sh
VOLUME ["/root/.sp1", "/data"]
EXPOSE 8787
ENTRYPOINT ["/app/entrypoint.sh"]
