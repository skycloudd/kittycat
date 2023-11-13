#!/bin/sh

CMD1="../target/release/kittycat"
CMD2=$CMD1

ROUNDS=1000

cutechess-cli \
-engine cmd=$CMD1 -engine cmd=$CMD2 \
-each st=0.5 timemargin=50 proto=uci \
-rounds $ROUNDS \
-repeat 2 \
-openings file=book.txt format=epd order=random \
-concurrency 5 \
-pgnout games.pgn
