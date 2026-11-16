# mod-llm-chatter's Python bridge for the beta realm: the Manager recipe's chatter.Dockerfile with
# the base image and every dependency pinned (llm-chatter.requirements.txt, hashed). The bridge
# source (chatter_source) is the staged, patched tools/ of the pinned module (realm.sh build).
FROM ghcr.io/astral-sh/uv:python3.11-bookworm-slim@sha256:4f5d923c9dcea037f57bda425dd209f3ec643da2f0b74227f68d09dab0b3bb36
COPY llm-chatter.requirements.txt /requirements.txt
RUN uv venv /venv && uv pip install --python /venv/bin/python --require-hashes --no-deps -r /requirements.txt
COPY --from=chatter_source /tools /app
WORKDIR /app
ENV PATH="/venv/bin:$PATH" PYTHONUNBUFFERED=1 PYTHONDONTWRITEBYTECODE=1
ENTRYPOINT ["/bin/sh", "/recipe/chatter.sh"]
