# syntax=docker/dockerfile:1
# Build context: clean core checkout with mod-playerbots under modules/.
FROM ubuntu:24.04 AS build
ARG BUILD_JOBS=2
ARG CORE_REV
ARG PLAYERBOTS_REV
ENV DEBIAN_FRONTEND=noninteractive CCACHE_DIR=/ccache CCACHE_MAXSIZE=1G
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential clang cmake ninja-build ccache git ca-certificates \
    default-libmysqlclient-dev libboost-all-dev libssl-dev libreadline-dev \
    zlib1g-dev libbz2-dev libncurses-dev liblzma-dev libmysql++-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /azerothcore
COPY . .
RUN set -eux; \
    git rev-parse HEAD; \
    test "$(git rev-parse HEAD)" = "$CORE_REV"; \
    git -C modules/mod-playerbots rev-parse HEAD; \
    test "$(git -C modules/mod-playerbots rev-parse HEAD)" = "$PLAYERBOTS_REV"; \
    git diff --name-status HEAD; \
    test -z "$(git diff --name-only HEAD)"; \
    git -C modules/mod-playerbots status --porcelain; \
    test -z "$(git -C modules/mod-playerbots status --porcelain)"
# Only selected module sources enter CMake. Keep core scaffold, remove every module directory.
RUN find modules -mindepth 1 -maxdepth 1 -type d -exec rm -rf {} +
COPY --from=module_sources . /azerothcore/modules/
RUN --mount=type=cache,target=/ccache,sharing=locked \
    cmake -S . -B build -G Ninja \
      -DCMAKE_INSTALL_PREFIX=/opt/acore -DCMAKE_BUILD_TYPE=Release \
      -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++ \
      -DCMAKE_C_COMPILER_LAUNCHER=ccache -DCMAKE_CXX_COMPILER_LAUNCHER=ccache \
      -DAPPS_BUILD=all -DTOOLS_BUILD=all -DSCRIPTS=static -DMODULES=static \
      -DWITH_WARNINGS=ON -DUSE_COREPCH=OFF -DUSE_SCRIPTPCH=OFF \
    && cmake --build build --parallel "$BUILD_JOBS" \
    && cmake --install build

FROM ubuntu:24.04 AS runtime
ARG CORE_REV
ARG PLAYERBOTS_REV
LABEL org.opencontainers.image.title="WrathPlus Playerbots arm64 spike" \
      org.opencontainers.image.revision="$CORE_REV" \
      au.com.artivus.playerbots.revision="$PLAYERBOTS_REV"
ENV DEBIAN_FRONTEND=noninteractive PATH=/opt/acore/bin:$PATH
RUN apt-get update && apt-get install -y --no-install-recommends \
    libmysqlclient21 libreadline8 libicu74 libssl3t64 zlib1g libbz2-1.0 \
    liblzma5 libncurses6 default-mysql-client ca-certificates util-linux \
    libboost-filesystem1.83.0 libboost-program-options1.83.0 \
    libboost-iostreams1.83.0 libboost-regex1.83.0 \
    libboost-thread1.83.0 libboost-atomic1.83.0 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 acore
COPY --from=build /opt/acore /opt/acore
# Core/module updaters resolve SQL relative to the compiled source directory.
COPY --from=build /azerothcore/data /azerothcore/data
COPY --from=build /azerothcore/modules /azerothcore/modules
RUN mkdir -p /state/config /state/logs /state/data \
    && chown -R acore:acore /state
USER acore
WORKDIR /state
CMD ["worldserver", "--help"]
