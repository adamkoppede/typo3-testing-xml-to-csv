# typo3 testing xml to csv

Command line application to render legacy xml
[typo3/testing-framework](https://github.com/TYPO3/testing-framework/tree/7.0.4)
fixtures as csv fixtures.

## Usage

```shell
cat test-fixture.xml \
    | docker run --rm -i ghcr.io/adamkoppede/typo3-testing-xml-to-csv \
    > test-fixture.csv
```

## Unsupported XML-Elements

Only the simplest xml fixtures are supported. Fixtures that make use of the
following features aren't:

- Comments inside the values
- CDATA
- Any special attributes like _ref_ and _is-NULL_
