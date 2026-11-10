FROM python@sha256:6771159cd4fa5d9bba1258caf0b82e6b73458c694d178ad97c5e925c2d0e1a91
LABEL org.opencontainers.image.title="GA4GH synthetic scenario API" \
      org.opencontainers.image.version="0.1.0" \
      org.opencontainers.image.source="https://github.com/superbsky/ga4gh-sdk"
WORKDIR /opt/ga4gh-testbed
COPY server.py ./server.py
COPY pack ./pack
USER 65532:65532
EXPOSE 8815
ENTRYPOINT ["python", "-B", "server.py", "--host", "0.0.0.0", "--port", "8815"]
