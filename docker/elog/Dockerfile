FROM alpine:3.21 AS builder

RUN apk add --no-cache build-base git openssl-dev
RUN git clone --recursive https://bitbucket.org/ritt/elog /build/elog && \
    cd /build/elog && make

FROM alpine:3.21

RUN apk add --no-cache libssl3 libcrypto3 libstdc++ libgcc

COPY --from=builder /build/elog/elogd /usr/local/sbin/elogd
COPY --from=builder /build/elog/resources /usr/local/elog/resources
COPY --from=builder /build/elog/themes /usr/local/elog/themes

RUN mkdir -p /usr/local/elog/logbooks

EXPOSE 8082

CMD ["/usr/local/sbin/elogd", "-p", "8082", "-c", "/usr/local/elog/elogd.cfg", "-d", "/usr/local/elog/logbooks"]
