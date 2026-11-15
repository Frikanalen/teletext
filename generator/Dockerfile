# ttx-pages, the studio page generator. Standard library only, so the image is
# just Python plus this package.
FROM python:3.13-slim

WORKDIR /src
COPY pyproject.toml README.md ./
COPY teletext_pages ./teletext_pages
RUN pip install --no-cache-dir . && rm -rf /src \
    && python -m teletext_pages.main --help >/dev/null

WORKDIR /
# nobody
USER 65534:65534
ENV PYTHONUNBUFFERED=1
ENTRYPOINT ["ttx-pages"]
# Mount the config here (a ConfigMap); the token comes from TTXD_TOKEN.
CMD ["/etc/ttx-pages/pages.toml"]
