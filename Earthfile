VERSION 0.8

ctx:
    FROM busybox
    RUN mkdir -p /tmp/docker-build
    SAVE ARTIFACT /tmp/docker-build

xwin:
    FROM DOCKERFILE -f xwin.dockerfile +ctx/docker-build
    SAVE IMAGE xwin:latest
