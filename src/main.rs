use std::{
    collections::HashMap,
    ffi::OsString,
    fs::File,
    io::{self, stdin, stdout, BufReader},
    str::from_utf8,
};

use anyhow::{Context, Result};
use clap::Parser;
use csv::Writer;
use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};

/// Converts a XML fixtures of typo3/testing-framework into a CSV fixtures.
#[derive(Parser, Debug)]
#[command(about, version)]
struct CommandLineArguments {
    /// File name of the file to write the output to.
    ///
    /// Output is written to [stdout] by default.
    #[arg(short, long)]
    output_file: Option<OsString>,
    /// File name of the file to read from.
    ///
    /// [stdin] is read by default.
    #[arg(short, long)]
    input_file: Option<OsString>,
}

fn main() -> Result<()> {
    let command_line_arguments = CommandLineArguments::parse();
    let mut reader = create_xml_reader(&command_line_arguments)?;
    let writer = create_csv_writer(&command_line_arguments)?;

    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "Error in input at position {}: {:?}",
                    reader.buffer_position(),
                    error
                ));
            }
            Ok(Event::Eof) => break,
            Ok(Event::Start(start_event)) => {
                if start_event.name().as_ref() != b"dataset" {
                    return Err(anyhow::anyhow!(
                        "Unexpected starting tag found at position {}. Expected to find a <dataset> starting tag.",
                        reader.buffer_position()
                        ));
                }
                return convert_dataset(reader, writer);
            }
            Ok(Event::Decl(_)) | Ok(Event::Text(_)) | Ok(Event::Comment(_)) => continue,
            token => {
                return Err(create_unexpected_token_error(
                    &reader,
                    format!(
                        "Erroring token is {:?}. Expected to find the start of a <dataset> element. ",
                        token
                        ).as_str(),
                ));
            }
        }
    }

    Err(anyhow::anyhow!("Input file is empty"))
}

fn create_unexpected_token_error<R: io::BufRead>(
    reader: &Reader<R>,
    expected_message: &str,
) -> anyhow::Error {
    anyhow::anyhow!(
        "Unexpected token in xml at position {}. {}",
        reader.buffer_position(),
        expected_message
    )
}

/// Convert the `<dataset>` element
///
/// [reader] must be inside the `<dataset>` element.
fn convert_dataset<R: io::BufRead, W: io::Write>(
    reader: Reader<R>,
    mut writer: Writer<W>,
) -> Result<()> {
    let dataset = read_dataset(reader)?;

    // Table occurrences need to be grouped because the table columns
    // definition is only read on the first occurrence of the table:
    // https://github.com/TYPO3/testing-framework/blob/7.0.4/Classes/Core/Functional/Framework/DataHandling/DataSet.php#L100-L102
    let mut tables: HashMap<&String, TableDataSet> = HashMap::new();

    for entry in dataset.iter() {
        let table = match tables.get_mut(&entry.name) {
            Some(table) => table,
            None => {
                let new_table = TableDataSet::new();
                tables.insert(&entry.name, new_table);
                tables.get_mut(&entry.name).unwrap()
            }
        };
        table.add_entry(entry)?;
    }

    let number_of_columns = match tables.values().map(|table| table.column_names.len()).max() {
        None => {
            eprintln!("Warning: <dataset> element is empty. Nothing will be written. ");
            return Ok(());
        }
        Some(0) => {
            eprintln!("Warning: No columns used in any element. Nothing will be written. ");
            return Ok(());
        }
        Some(number_of_columns) => number_of_columns,
    };

    const EMPTY_STR: &'static str = "";
    let mut write_buffer: Vec<&str> = vec![EMPTY_STR; number_of_columns + 1];

    for (table_name, table_data_set) in tables {
        write_buffer.fill_with(|| EMPTY_STR);
        write_buffer[0] = table_name;

        writer
            .write_record(&write_buffer)
            .context("Failed to write csv table name row")?;

        let table_group_column_len = table_data_set.column_names.len();
        write_buffer[1..table_group_column_len + 1]
            .copy_from_slice(&table_data_set.column_names[..]);
        write_buffer[0] = EMPTY_STR;

        writer
            .write_record(&write_buffer)
            .context("Failed to write csv column header row")?;

        for entry in table_data_set.entries {
            write_buffer.fill_with(|| EMPTY_STR);

            table_data_set.column_names.iter().enumerate().for_each(
                |(column_index, column_name)| {
                    write_buffer[column_index + 1] = match entry.cells.get(column_name.to_owned()) {
                        Some(column_payload) => column_payload,
                        None => "",
                    }
                },
            );

            writer
                .write_record(&write_buffer)
                .context("Failed to write csv data row")?;
        }
    }

    writer.flush().context("Failed to flush csv")?;

    Ok(())
}

struct TableDataSet<'a> {
    column_names: Vec<&'a str>,
    entries: Vec<&'a TableEntry>,
}

impl<'a> TableDataSet<'a> {
    pub fn new() -> Self {
        return Self {
            column_names: Vec::new(),
            entries: Vec::new(),
        };
    }

    pub fn add_entry(&mut self, entry: &'a TableEntry) -> Result<()> {
        entry.cells.keys().for_each(|cell_column_name| {
            if !self
                .column_names
                .iter()
                .any(|column_name| column_name == cell_column_name)
            {
                self.column_names.push(cell_column_name);
            }
        });

        // Swap the first column with the required "uid" column, since
        // the first value field must never be an empty string
        // https://github.com/TYPO3/testing-framework/blob/c9f5d6e25998ba2373612fb1e32846298f5cadf7/Classes/Core/Functional/Framework/DataHandling/DataSet.php#L228
        const UID_COLUMN_NAME: &'static str = "uid";
        let uid_column_position = self
            .column_names
            .iter()
            .position(|column_name| *column_name == UID_COLUMN_NAME)
            .context("Found a record with uid column")?;
        if uid_column_position != 0 {
            self.column_names.swap(0, uid_column_position);
        }

        self.entries.push(entry);
        Ok(())
    }
}

/// Consumes the entire `<dataset>` element.
///
/// [reader] must be inside the `<dataset>` element.
fn read_dataset<R: io::BufRead>(mut reader: Reader<R>) -> Result<Vec<TableEntry>> {
    let mut data_list: Vec<TableEntry> = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::End(end_event)) => {
                if end_event.name().as_ref() != b"dataset" {
                    return Err(create_unexpected_token_error(
                        &reader,
                        "Expected to find ending tag </dataset> or start of a new table element. ",
                    ));
                }

                break;
            }
            Ok(Event::Start(start_event)) => {
                let table_position = reader.buffer_position();
                let table_data = read_entry(&mut reader, start_event).with_context(|| {
                    format!("Could not read table at position {}.", table_position)
                })?;
                data_list.push(table_data)
            }
            Ok(Event::Text(_)) | Ok(Event::Comment(_)) => continue,
            _ => {
                return Err(create_unexpected_token_error(
                    &reader,
                    "Expected to find the start of a table element or the end of the dataset. ",
                ));
            }
        }
    }

    Ok(data_list)
}

fn read_entry<R: io::BufRead>(
    reader: &mut Reader<R>,
    start_event: BytesStart,
) -> Result<TableEntry> {
    let mut table_entry =
        TableEntry::try_from(&start_event).context("Failed to read table start")?;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::End(end_event)) => {
                if end_event.name().as_ref() != start_event.name().as_ref() {
                    return Err(create_unexpected_token_error(
                        &reader,
                        format!(
                            "Expected to find ending tag </{}> or start of a new table element. ",
                            table_entry.name
                        )
                        .as_str(),
                    ));
                }

                break;
            }
            Ok(Event::Empty(empty_cell_event)) => {
                let cell_position = reader.buffer_position();
                let cell_name = from_utf8(empty_cell_event.name().as_ref())
                    .with_context(|| {
                        format!(
                            "Could not decode cell name at position {} inside <{}> using utf8",
                            cell_position, table_entry.name
                        )
                    })?
                    .to_owned();

                if table_entry.cells.contains_key(&cell_name) {
                    eprintln!(
                        "Warning: Duplicated cell {} in table {} at position {}",
                        cell_name, table_entry.name, cell_position
                    );
                }

                table_entry.cells.insert(cell_name, "".to_string());
            }
            Ok(Event::Start(cell_start_event)) => {
                let cell_position = reader.buffer_position();
                let cell_name = from_utf8(cell_start_event.name().as_ref())
                    .with_context(|| {
                        format!(
                            "Could not decode cell name at position {} inside <{}> using utf8",
                            cell_position, table_entry.name
                        )
                    })?
                    .to_owned();
                let (close_event_was_reached, cell_payload) = match reader.read_event_into(&mut buf) {
                    Ok(Event::Text(cell_text_event)) => {
                        (false, cell_text_event.unescape().with_context(|| {
                            format!(
                                "Failed to parse contents of cell at position {} in cell <{}> of table <{}>",
                                reader.buffer_position(),
                                cell_name,
                                table_entry.name
                            )
                        })?.as_ref().to_owned())
                    },
                    Ok(Event::End(cell_close_event)) => {
                        if cell_close_event.name().as_ref() != cell_name.as_str().as_bytes() {
                            return Err(create_unexpected_token_error(
                                &reader,
                                format!(
                                    "Got {:?}. Expected to find </{}>", 
                                    cell_close_event,
                                    cell_name
                                ).as_str(),
                            ));
                        }
                        (true, "".to_owned())
                    }
                    _ => {
                        return Err(anyhow::anyhow!(
                            "Could not read cell contents at position {} inside <{}>",
                            cell_position,
                            table_entry.name
                        ));
                    }
                };
                if !close_event_was_reached {
                    match reader.read_event_into(&mut buf) {
                        Ok(Event::End(cell_close_event)) => {
                            if cell_close_event.name().as_ref() != cell_name.as_str().as_bytes() {
                                return Err(create_unexpected_token_error(
                                    &reader,
                                    format!(
                                        "Got {:?}. Expected to find </{}>",
                                        cell_close_event, cell_name
                                    )
                                    .as_str(),
                                ));
                            }
                        }
                        token => {
                            return Err(create_unexpected_token_error(
                                &reader,
                                format!("Got {:?}. Expected to find </{}>", token, cell_name)
                                    .as_str(),
                            ));
                        }
                    }
                }

                if table_entry.cells.contains_key(&cell_name) {
                    eprintln!(
                        "Warning: Duplicated cell {} in table {} at position {}",
                        cell_name, table_entry.name, cell_position
                    );
                }

                table_entry.cells.insert(cell_name, cell_payload);
            }
            Ok(Event::Text(_)) | Ok(Event::Comment(_)) => continue,
            _ => {
                return Err(create_unexpected_token_error(
                    &reader,
                    "Expected to find the start of a table element or the end of the dataset. ",
                ));
            }
        }
    }

    Ok(table_entry)
}

struct TableEntry {
    pub name: String,
    pub cells: HashMap<String, String>,
}
impl TryFrom<&BytesStart<'_>> for TableEntry {
    type Error = anyhow::Error;

    fn try_from(value: &BytesStart) -> std::prelude::v1::Result<Self, Self::Error> {
        let name = value.name();
        let str = from_utf8(name.as_ref()).context("Failed to decode tag bytes using utf8.")?;

        Ok(Self {
            name: str.to_owned(),
            cells: HashMap::new(),
        })
    }
}

fn create_xml_reader(
    command_line_arguments: &CommandLineArguments,
) -> Result<Reader<BufReader<Box<dyn io::Read>>>> {
    let input_fd: Box<dyn io::Read> = match &command_line_arguments.input_file {
        None => Box::new(stdin()),
        Some(input_file_name) => Box::new(File::open(input_file_name).with_context(|| {
            format!(
                "Failed to open input file {}.",
                input_file_name.to_string_lossy()
            )
        })?),
    };

    let bufferred = BufReader::new(input_fd);

    Ok(Reader::from_reader(bufferred))
}

fn create_csv_writer(
    command_line_arguments: &CommandLineArguments,
) -> Result<Writer<Box<dyn io::Write>>> {
    let output_fd: Box<dyn io::Write> = match &command_line_arguments.output_file {
        None => Box::new(stdout()),
        Some(output_file_name) => Box::new(
            File::options()
                .append(true)
                .create(true)
                .open(output_file_name)
                .with_context(|| {
                    format!(
                        "Failed to open output file {}.",
                        output_file_name.to_string_lossy()
                    )
                })?,
        ),
    };

    Ok(Writer::from_writer(output_fd))
}
