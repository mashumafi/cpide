FROM alpine:3.11.6
RUN apk add --no-cache cargo g++ git npm

ENV WORKAREA /workarea
RUN mkdir -p ${WORKAREA}
WORKDIR ${WORKAREA}

RUN git clone --recursive https://github.com/ajaxorg/ace.git static_html/ace/ && \
    git clone --recursive https://github.com/jquery/jquery.git static_html/jquery/

RUN cd static_html/jquery && npm run-script build

COPY . ${WORKAREA}/

ENTRYPOINT cargo run --release
