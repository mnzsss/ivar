FROM alpine:3.20
RUN apk add --no-cache tinyproxy
USER 65534:65534
EXPOSE 8888
ENTRYPOINT ["tinyproxy", "-d", "-c", "/etc/tinyproxy/tinyproxy.conf"]
