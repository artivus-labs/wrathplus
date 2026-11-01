# Bundled upstream Python service; no packages installed into the host environment.
FROM ghcr.io/astral-sh/uv:python3.11-bookworm-slim
COPY --from=chatter_source /tools/requirements.txt /requirements.txt
RUN uv venv /venv && uv pip install --python /venv/bin/python -r /requirements.txt
COPY --from=chatter_source /tools /app
WORKDIR /app
ENV PATH="/venv/bin:$PATH" PYTHONUNBUFFERED=1 PYTHONDONTWRITEBYTECODE=1
ENTRYPOINT ["/bin/sh", "/recipe/chatter.sh"]
