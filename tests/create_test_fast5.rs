/// Tests for Fast5 (Nanopore HDF5) file reading.
/// Test files created with Python h5py.
use fastqc_rust::sequence::fast5::Fast5File;
use fastqc_rust::sequence::SequenceFile;

#[test]
fn test_fast5_single_read() {
    let mut reader =
        Fast5File::open("tests/data/single_read.fast5").expect("Failed to open single-read Fast5");

    assert_eq!(reader.name(), "single_read.fast5");

    let seq = reader
        .next()
        .expect("Should have a sequence")
        .expect("Should parse OK");
    assert_eq!(seq.id, "@read_001");
    assert_eq!(
        std::str::from_utf8(&seq.sequence).unwrap(),
        "ACGTACGTACGTACGT"
    );
    assert_eq!(
        std::str::from_utf8(&seq.quality).unwrap(),
        "IIIIIIIIIIIIIIII"
    );

    assert!(reader.next().is_none(), "Should be EOF after 1 read");
}

#[test]
fn test_fast5_multi_read() {
    let mut reader =
        Fast5File::open("tests/data/multi_read.fast5").expect("Failed to open multi-read Fast5");

    let seq1 = reader.next().expect("Should have seq 1").expect("Parse OK");
    assert_eq!(seq1.id, "@read_001");
    assert_eq!(
        std::str::from_utf8(&seq1.sequence).unwrap(),
        "ACGTACGTACGTACGT"
    );

    let seq2 = reader.next().expect("Should have seq 2").expect("Parse OK");
    assert_eq!(seq2.id, "@read_002");
    assert_eq!(
        std::str::from_utf8(&seq2.sequence).unwrap(),
        "TTTTAAAACCCCGGGG"
    );

    let seq3 = reader.next().expect("Should have seq 3").expect("Parse OK");
    assert_eq!(seq3.id, "@read_003");
    assert_eq!(
        std::str::from_utf8(&seq3.sequence).unwrap(),
        "GGGGCCCCTTTTAAAA"
    );

    assert!(reader.next().is_none(), "Should be EOF after 3 reads");
}

#[test]
fn test_fast5_progress() {
    let mut reader = Fast5File::open("tests/data/multi_read.fast5").expect("Failed to open");

    assert_eq!(reader.percent_complete(), 0.0);

    reader.next(); // read 1
    let pct = reader.percent_complete();
    assert!(
        pct > 0.0 && pct <= 50.0,
        "Progress should be ~33%, got {}",
        pct
    );

    reader.next(); // read 2
    reader.next(); // read 3
    assert_eq!(reader.percent_complete(), 100.0);
}

#[test]
fn test_fast5_end_to_end() {
    // Run the full analysis pipeline on a Fast5 file
    use fastqc_rust::config::FastQCConfig;
    use fastqc_rust::modules;
    use fastqc_rust::report;

    let config = FastQCConfig::default();
    let limits = config.load_limits().unwrap();

    let mut seq_file = Fast5File::open("tests/data/single_read.fast5").unwrap();
    let file_display_name = seq_file.name().to_string();

    let mut mods = modules::create_modules(&config, &limits);
    for module in mods.iter_mut() {
        module.set_filename(&file_display_name);
    }

    loop {
        match seq_file.next() {
            Some(Ok(seq)) => {
                for module in mods.iter_mut() {
                    module.process_sequence(&seq);
                }
            }
            Some(Err(e)) => panic!("Error: {}", e),
            None => break,
        }
    }

    for module in mods.iter_mut() {
        module.finalize();
    }

    // Generate text report
    let mut data_buf = Vec::new();
    report::text::write_fastqc_data(&mods, &mut data_buf).unwrap();
    let data_text = String::from_utf8(data_buf).unwrap();

    // Verify it contains expected content
    assert!(data_text.contains("##FastQC"));
    assert!(data_text.contains("Basic Statistics"));
    assert!(data_text.contains("single_read.fast5"));
    assert!(data_text.contains("Total Sequences\t1"));
    assert!(data_text.contains("Sequence length\t16"));
}
