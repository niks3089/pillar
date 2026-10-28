# A disposable "validator host" for the system suite: real systemd as PID 1, so
# install-node.sh, sudoers, pillar-agent.service and the validator unit all run
# exactly as on a bare-metal node. BASE_IMAGE should match the glibc the agent
# was built against (the harness passes the host's Ubuntu release).
ARG BASE_IMAGE=ubuntu:24.04
FROM ${BASE_IMAGE}

ENV DEBIAN_FRONTEND=noninteractive container=docker
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      systemd systemd-sysv dbus sudo curl ca-certificates iproute2 procps util-linux \
 && apt-get clean && rm -rf /var/lib/apt/lists/* \
 # Units that make no sense (or hang) inside a container.
 && systemctl mask systemd-modules-load.service \
 && rm -f /lib/systemd/system/multi-user.target.wants/* \
          /etc/systemd/system/*.wants/* \
          /lib/systemd/system/local-fs.target.wants/* \
          /lib/systemd/system/sockets.target.wants/*udev* \
          /lib/systemd/system/sockets.target.wants/*initctl* \
          /lib/systemd/system/sysinit.target.wants/systemd-tmpfiles-setup* \
          /lib/systemd/system/systemd-update-utmp*

STOPSIGNAL SIGRTMIN+3
CMD ["/lib/systemd/systemd"]
