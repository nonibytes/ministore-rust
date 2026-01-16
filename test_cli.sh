#!/bin/bash
set -e
BIN=target/debug/ministore
DB=test_cli.db
rm -f $DB*

echo "Creating index..."
$BIN index create -i $DB --field title:text --field count:number

echo "Putting document..."
$BIN put -i $DB -p "/doc1" --set title="hello world" --set count=10

echo "Searching..."
$BIN search -i $DB -w "title:hello"

echo "Getting document..."
$BIN get -i $DB -p "/doc1"

echo "Deleting document..."
$BIN delete -i $DB -p "/doc1"

echo "Searching again (should be empty)..."
$BIN search -i $DB -w "title:hello"

echo "Done."
rm -f $DB*
