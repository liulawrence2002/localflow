//! Judges from the words alone whether a speaker has finished their turn.
//!
//! In hands-free conversation the user's speech is transcribed at each pause. A transcript
//! that reads as a finished thought lets the assistant answer after a short silence; one that
//! trails off on a word that needs something after it makes it wait longer. Cutting someone
//! off mid-thought is much worse than answering a little late, so `Complete` is only returned
//! on positive evidence and everything doubtful is `Unsure`.
//!
//! The recogniser writes its own punctuation, and at a pause that punctuation is unreliable:
//! a full stop often marks a mere pause, and a true ending sometimes has none. So the words
//! decide and punctuation is weak evidence. A full stop never rescues "that we could.", a
//! well-formed question needs no question mark, and only a plain statement leans on its stop.
//!
//! English only. Fixed word lists and rules: deterministic, nothing logged, nothing stored.

/// How finished a transcript reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completeness {
    /// Reads as a finished question, statement or command: answer after a short silence.
    Complete,
    /// Clearly unfinished: trails off on a word that needs something after it.
    Incomplete,
    /// Could be either: use the ordinary wait.
    Unsure,
}

/// Judge the transcript of everything the user has said in this turn so far.
pub fn judge_transcript(transcript: &str) -> Completeness {
    let mut tokens = tokenize(transcript);
    if tokens.is_empty() {
        return Completeness::Unsure;
    }
    let ending = ending_of(transcript);
    if ending == Ending::Continuing || has_unclosed_pair(transcript) {
        return Completeness::Incomplete;
    }

    // A closing "please" says nothing about completeness: "can you please" is as open as
    // "can you".
    while tokens.len() > 1 && tokens[tokens.len() - 1].text == "please" {
        tokens.pop();
    }

    // Only the last sentence can be unfinished, so only it is judged.
    let start = tokens[..tokens.len() - 1]
        .iter()
        .rposition(|token| token.stop)
        .map_or(0, |index| index + 1);
    let segment = without_repeats(&tokens[start..]);
    let words: Vec<&str> = segment.iter().map(|token| token.text.as_str()).collect();
    let marked = ending != Ending::None;

    if is_conventional(&words, marked) {
        return Completeness::Complete;
    }
    if ends_on_filler(&words) {
        return Completeness::Incomplete;
    }

    let lead = lead_in_len(&words);
    let answer_lead = words[..lead].iter().any(|word| ANSWER_WORDS.contains(word));
    let core = &words[lead..];
    if core.is_empty() {
        // Nothing but lead-ins: "so", "and then", "okay so" are waiting for the rest.
        let last = words[words.len() - 1];
        return if HESITANT_LEADS.contains(&last) || FILLERS.contains(&last) {
            Completeness::Incomplete
        } else {
            Completeness::Unsure
        };
    }

    let shape = shape_of(core);
    match tail_of(core, shape, ending, answer_lead) {
        Tail::Dangling => return Completeness::Incomplete,
        Tail::Open => return Completeness::Unsure,
        Tail::Closed => {}
    }

    let count = core.len();
    let object_pronoun_last = OBJECT_WORDS.contains(&core[count - 1]);
    let complete = match shape {
        Shape::WhQuestion => count >= 3 || (count == 2 && (marked || is_contraction(core[0]))),
        // Three unmarked words are enough when the verb is "be" and the subject a pronoun
        // ("are you there"); "can you help" may still be on its way to an object.
        Shape::Inversion => {
            count >= 4
                || (count == 3 && (marked || (is_be(core[0]) && PRONOUNISH.contains(&core[1]))))
        }
        Shape::Imperative => {
            let needs_direction = NEEDS_DIRECTION.contains(&core[0])
                && !core[1..].iter().any(|word| DIRECTIONS.contains(word));
            !needs_direction && (count >= 3 || (count == 2 && (marked || object_pronoun_last)))
        }
        Shape::Request => count >= 4,
        // A clause that opens with "if" or "when" needs a main clause after it.
        Shape::Subordinate => marked && has_main_clause(&segment[lead..]),
        // A statement can always go on, so it needs the recogniser's stop as well.
        Shape::Declarative => {
            // Two words are enough after "yes"/"no" ("No, it isn't.") or when the first is a
            // pronoun subject ("It depends.", "It's over.").
            let short_ok = answer_lead || PRONOUNISH.contains(&base_of(core[0]));
            marked && has_verb(core) && count >= if short_ok { 2 } else { 3 }
        }
    };
    if complete {
        Completeness::Complete
    } else if shape == Shape::Subordinate
        && !marked
        && STRONG_SUBORDINATORS.contains(&core[0])
        && !has_main_clause(&segment[lead..])
    {
        // "When I get home" with nothing after it is half a sentence.
        Completeness::Incomplete
    } else {
        Completeness::Unsure
    }
}

/// One spoken word, lower case, with the punctuation the recogniser put after it.
#[derive(Debug, Clone)]
struct Token {
    text: String,
    comma: bool,
    /// Followed by `.`, `?` or `!` that ends a sentence (not an abbreviation or initial).
    stop: bool,
}

/// What the transcript's final punctuation says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    None,
    Stop,
    Question,
    /// A comma, colon, dash or ellipsis: the speaker is visibly mid-sentence.
    Continuing,
}

/// How the last sentence is built, which decides how much evidence it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    WhQuestion,
    /// Auxiliary first: "can you ...", "is it ...".
    Inversion,
    Imperative,
    /// "I want ...", "I'd like ...": a command in a statement's clothes.
    Request,
    /// Opens with "if", "when", "because" and the like.
    Subordinate,
    Declarative,
}

/// What the final word says about the sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tail {
    /// Needs something after it.
    Dangling,
    /// Could end here or go on.
    Open,
    /// A plausible last word.
    Closed,
}

/// Hesitation sounds that carry no words. Also used to tell a listener's "mm-hmm" from an
/// interruption (`barge_in`).
pub const FILLERS: &[&str] = &[
    "um", "uh", "er", "erm", "uhm", "umm", "uhh", "ah", "hmm", "hm", "mm", "eh",
];
/// Opening words that carry no content and, alone, promise more.
const HESITANT_LEADS: &[&str] = &[
    "so",
    "and",
    "but",
    "or",
    "well",
    "like",
    "then",
    "actually",
    "basically",
    "also",
    "anyway",
    "now",
    "just",
];
/// Opening words that carry no content but do not, alone, promise more.
const NEUTRAL_LEADS: &[&str] = &[
    "okay", "ok", "hey", "hi", "hello", "oh", "yes", "yeah", "yep", "no", "nope", "sure", "right",
    "alright", "please",
];
const ANSWER_WORDS: &[&str] = &["yes", "yeah", "yep", "no", "nope", "sure", "okay", "ok"];
/// "No" before these is a determiner, not an answer: "no one knows".
const AFTER_DETERMINER_NO: &[&str] = &[
    "one", "more", "idea", "problem", "way", "matter", "longer", "need", "wonder", "doubt",
];

/// Whole utterances that are finished however they are punctuated. Apostrophes removed.
const CONVENTIONAL: &[&str] = &[
    "yes",
    "yeah",
    "yep",
    "yup",
    "yes please",
    "no",
    "nope",
    "nah",
    "no thanks",
    "no thank you",
    "ok",
    "okay",
    "okay then",
    "ok then",
    "alright",
    "all right",
    "alright then",
    "sure",
    "sure thing",
    "of course",
    "thanks",
    "thank you",
    "thanks a lot",
    "thank you very much",
    "thank you so much",
    "thanks so much",
    "stop",
    "cancel",
    "quiet",
    "be quiet",
    "goodbye",
    "good bye",
    "bye",
    "see you",
    "see you later",
    "good night",
    "good morning",
    "hello",
    "hi",
    "hey there",
    "hello there",
    "hi there",
    "never mind",
    "nevermind",
    "forget it",
    "thats all",
    "that is all",
    "that will be all",
    "thats it",
    "thats enough",
    "that is enough",
    "thats right",
    "thats fine",
    "thats okay",
    "thats ok",
    "thats great",
    "go on",
    "go ahead",
    "continue",
    "keep going",
    "carry on",
    "repeat",
    "repeat that",
    "say that again",
    "say it again",
    "come again",
    "pardon",
    "again",
    "one more time",
    "next",
    "previous",
    "back",
    "go back",
    "skip",
    "exit",
    "start over",
    "from the top",
    "i dont know",
    "i dunno",
    "i think so",
    "i dont think so",
    "i guess so",
    "i hope so",
    "i hope not",
    "i guess not",
    "not really",
    "not yet",
    "not now",
    "maybe later",
    "no way",
    "got it",
    "cool",
    "great",
    "perfect",
    "nice",
    "awesome",
    "fine",
    "good",
    "correct",
    "exactly",
    "absolutely",
    "definitely",
    "right",
    "sounds good",
    "makes sense",
    "me too",
    "me neither",
    "same here",
    "maybe",
    "done",
    "im done",
    "were done",
    "we are done",
    "youre welcome",
    "no problem",
    "will do",
    "why not",
    "what did you say",
    "end conversation",
    "end the conversation",
    "stop listening",
    "stop the conversation",
    "exit conversation",
    "help",
    "really",
    "sorry",
    "excuse me",
    "wow",
];
/// Finished only when the recogniser also heard an ending: unmarked, "what" and "I know"
/// are just as likely the first words of something longer.
const CONVENTIONAL_IF_MARKED: &[&str] = &[
    "what", "why", "how", "who", "where", "when", "which", "huh", "i know", "i see", "so what",
    "how come", "what for",
];
const MAX_CONVENTIONAL_WORDS: usize = 5;

const WH_WORDS: &[&str] = &[
    "what", "who", "where", "when", "why", "how", "which", "whose", "whom",
];
/// Question words that can stand for the missing object of the final verb: "what do you
/// want" is whole, "how do I make" is not.
const OBJECT_WH: &[&str] = &["what", "which", "who", "whom", "whose", "whatever"];
/// Mid-sentence words that can do the same in a statement: "that's all I need".
const FRONTED_HINTS: &[&str] = &[
    "what",
    "whatever",
    "which",
    "who",
    "all",
    "everything",
    "anything",
    "something",
    "thing",
    "things",
    "one",
    "ones",
];
const HOW_COMPANIONS: &[&str] = &[
    "many", "much", "long", "far", "old", "often", "big", "tall", "fast", "soon", "late", "hard",
    "heavy", "deep", "high", "likely", "come", "about", "else", "exactly",
];
/// After "what"/"who"/"which" these make a clause ("what I want"), not a question.
const CLAUSE_SUBJECTS: &[&str] = &[
    "i", "you", "we", "they", "he", "she", "it", "the", "a", "an", "my", "your", "our", "their",
    "this", "that", "these", "those", "there",
];
const SUBORDINATORS: &[&str] = &[
    "if", "when", "while", "because", "cause", "since", "although", "though", "unless", "until",
    "before", "after", "once", "as", "whenever", "wherever", "whether", "where", "why", "how",
    "even", "in",
];
/// Openers that so rarely stand alone that a lone clause after them is unfinished.
const STRONG_SUBORDINATORS: &[&str] = &["if", "when", "while", "unless", "although", "whenever"];

const BE_FORMS: &[&str] = &["am", "is", "are", "was", "were"];
const MODALS: &[&str] = &[
    "can", "could", "will", "would", "should", "shall", "may", "might", "must",
];
const OTHER_AUXILIARIES: &[&str] = &[
    "do", "does", "did", "have", "has", "had", "be", "been", "being",
];

const SUBJECT_PRONOUNS: &[&str] = &["i", "we", "they", "he", "she"];
/// Words that can be the subject of a short answer such as "I do" or "it is".
const PRONOUNISH: &[&str] = &[
    "i", "we", "they", "he", "she", "you", "it", "that", "this", "there", "those", "these",
];
/// Words that complete a verb or preposition before them.
const OBJECT_WORDS: &[&str] = &[
    "it",
    "this",
    "that",
    "these",
    "those",
    "you",
    "them",
    "me",
    "him",
    "her",
    "us",
    "one",
    "something",
    "anything",
    "everything",
    "nothing",
    "here",
    "there",
];

/// Never a last word.
const ALWAYS_DANGLING: &[&str] = &[
    // conjunctions and complementisers
    "and",
    "but",
    "or",
    "nor",
    "because",
    "cause",
    "cuz",
    "although",
    "whereas",
    "if",
    "unless",
    "whether",
    "as",
    "than",
    "whenever",
    "wherever",
    "plus",
    "versus",
    "vs",
    "since",
    // prepositions that are never left stranded
    "except",
    "per",
    "via",
    "during",
    "between",
    "among",
    "amongst",
    "against",
    "towards",
    "toward",
    "upon",
    "despite",
    "including",
    "regarding",
    "until",
    "till",
    "onto",
    "unlike",
    // articles, determiners, possessives
    "the",
    "a",
    "an",
    "my",
    "your",
    "our",
    "their",
    "its",
    "his",
    "every",
    "several",
    "various",
    // intensifiers and reduced forms that lean on the next word
    "very",
    "quite",
    "rather",
    "just",
    "even",
    "only",
    "almost",
    "nearly",
    "especially",
    "particularly",
    "such",
    "extremely",
    "fairly",
    "roughly",
    "approximately",
    "kinda",
    "sorta",
    "gonna",
    "gotta",
    "wanna",
    "lemme",
    "gimme",
];
/// Prepositions that a question or relative clause may leave at the end.
const STRANDABLE: &[&str] = &[
    "for", "with", "at", "from", "about", "into", "after", "under",
];
/// Verbs and adjectives that take such a preposition: "what are you waiting for".
const STRAND_HOSTS: &[&str] = &[
    "look",
    "wait",
    "ask",
    "pay",
    "talk",
    "come",
    "came",
    "go",
    "live",
    "work",
    "send",
    "give",
    "speak",
    "listen",
    "belong",
    "refer",
    "think",
    "hear",
    "heard",
    "made",
    "used",
    "called",
    "known",
    "afraid",
    "interested",
    "good",
    "bad",
    "worried",
    "excited",
    "up",
    "play",
    "played",
    "stay",
    "stayed",
    "sit",
    "deal",
    "agree",
    "depend",
    "rely",
    "apply",
    "connected",
    "related",
    "married",
    "addressed",
    "sent",
    "gave",
    "given",
    "talked",
    "spoke",
    "went",
    "lived",
    "worked",
    "stand",
    "care",
    "dream",
    "worry",
    "point",
    "aim",
    "vote",
    "search",
    "meant",
    "mean",
];
const OF_HOSTS: &[&str] = &[
    "made", "think", "thinking", "thought", "afraid", "scared", "heard", "hear", "proud", "tired",
    "sick", "capable", "aware", "dream", "dreaming", "consist", "speak", "speaking",
];
/// Before a final "to" these make it the start of an infinitive: "I want to".
const INFINITIVE_TAKERS: &[&str] = &[
    "going",
    "trying",
    "planning",
    "hoping",
    "supposed",
    "want",
    "need",
    "have",
    "has",
    "had",
    "like",
    "love",
    "able",
    "how",
    "what",
    "where",
    "when",
    "way",
    "time",
    "order",
    "got",
    "about",
    "wants",
    "needs",
    "wanted",
    "needed",
    "tried",
    "try",
    "plan",
    "hope",
    "decided",
    "decide",
    "wish",
    "forgot",
    "forget",
    "remember",
    "learn",
    "learning",
    "ready",
    "allowed",
    "ought",
    "seem",
    "seems",
    "tend",
    "start",
    "started",
    "begin",
    "began",
    "continue",
    "expect",
    "prefer",
    "promise",
    "refuse",
    "manage",
    "fail",
    "afford",
    "chance",
    "ability",
    "right",
    "reason",
    "place",
    "thing",
    "something",
    "anything",
    "nothing",
    "lot",
    "much",
    "enough",
    "easy",
    "hard",
    "difficult",
    "possible",
    "impossible",
    "important",
    "necessary",
    "nice",
    "good",
    "best",
    "better",
    "free",
    "happy",
    "glad",
    "sorry",
    "sure",
    "likely",
    "wanting",
    "needing",
    "meaning",
    "starting",
    "beginning",
    "attempting",
    "aiming",
    "looking",
    "waiting",
    "getting",
    "having",
    "you",
    "me",
    "us",
    "them",
    "him",
    "her",
];
/// After these, a final "in" or "on" is a particle: "what's going on", "come in".
const IN_ON_HOSTS: &[&str] = &[
    "going", "go", "goes", "come", "came", "coming", "went", "log", "logged", "sign", "signed",
    "move", "moving", "carry", "later", "early", "tune", "zoom", "drop", "jump", "dig", "give",
    "kick", "sleep", "stay", "check", "keep", "kept",
];
const PARTICLES: &[&str] = &[
    "up", "down", "off", "out", "back", "away", "over", "ahead", "together", "apart", "forward",
];
const LOOSE_PARTICLES: &[&str] = &["around", "through", "along"];
/// Verbs whose particle is usually followed by an object: "turn off" (the lights).
const PHRASAL_TRANSITIVE: &[&str] = &[
    "turn", "turning", "pick", "look", "find", "figure", "set", "put", "take", "bring", "fill",
    "write", "point", "cut", "switch", "check", "clean", "sort", "rule", "try", "send", "hand",
    "lay", "leave", "make", "cross", "tear", "throw", "blow", "mark", "wipe", "pull", "type",
    "read", "print", "carry", "add", "sum", "open", "wrap", "drop",
];
const BY_HOSTS: &[&str] = &[
    "stop", "come", "go", "drop", "stand", "swing", "pass", "get",
];

/// Before a final "that" these make it a complementiser: "I think that".
const THAT_TAKERS: &[&str] = &[
    "think",
    "thought",
    "thinking",
    "say",
    "said",
    "saying",
    "says",
    "know",
    "knew",
    "believe",
    "hope",
    "hoping",
    "mean",
    "means",
    "guess",
    "suppose",
    "sure",
    "glad",
    "afraid",
    "so",
    "such",
    "fact",
    "idea",
    "feel",
    "felt",
    "heard",
    "hear",
    "told",
    "tell",
    "realize",
    "realized",
    "noticed",
    "notice",
    "assume",
    "understand",
    "wondering",
    "seems",
    "appears",
    "is",
    "was",
    "be",
    "been",
    "and",
    "but",
    "now",
    "given",
    "except",
    "in",
    "see",
    "show",
    "shows",
    "mention",
    "mentioned",
    "forgot",
    "forget",
    "agree",
    "decided",
    "found",
];
/// With an object between, these also take a "that" clause: "he told me that".
const TELL_VERBS: &[&str] = &[
    "told",
    "tell",
    "tells",
    "telling",
    "promise",
    "promised",
    "show",
    "showed",
    "remind",
    "reminded",
    "assure",
    "convinced",
    "inform",
    "informed",
];
/// Before a final "this"/"that" these make it a pronoun: "I don't like that".
const DEMONSTRATIVE_HOSTS: &[&str] = &[
    "like",
    "do",
    "did",
    "does",
    "doing",
    "done",
    "read",
    "repeat",
    "want",
    "need",
    "saw",
    "about",
    "of",
    "for",
    "with",
    "at",
    "on",
    "to",
    "from",
    "by",
    "than",
    "get",
    "got",
    "try",
    "love",
    "hate",
    "stop",
    "take",
    "have",
    "had",
    "open",
    "close",
    "delete",
    "send",
    "fix",
    "change",
    "use",
    "check",
    "play",
    "add",
    "remove",
    "not",
    "just",
    "all",
    "exactly",
    "call",
    "hold",
    "keep",
    "leave",
    "make",
    "made",
    "put",
    "watch",
    "buy",
    "appreciate",
    "enjoy",
    "handle",
    "answer",
    "explain",
    "consider",
    "catch",
    "miss",
    "find",
    "prefer",
    "into",
    "after",
    "before",
    "without",
    "beyond",
    "besides",
];
const SO_HOSTS: &[&str] = &[
    "think", "hope", "guess", "suppose", "believe", "say", "said", "afraid", "do", "did", "doing",
    "even", "not",
];
const NOT_HOSTS: &[&str] = &[
    "why",
    "think",
    "hope",
    "guess",
    "course",
    "rather",
    "afraid",
    "absolutely",
    "certainly",
    "probably",
    "maybe",
    "or",
];
const MORE_HOSTS: &[&str] = &[
    "me",
    "some",
    "any",
    "no",
    "once",
    "much",
    "lot",
    "bit",
    "little",
    "want",
    "need",
    "anything",
    "something",
    "nothing",
    "say",
    "do",
    "know",
    "one",
    "few",
    "many",
];
const ONE_HOSTS: &[&str] = &[
    "this", "that", "which", "another", "first", "second", "third", "good", "new", "last", "next",
    "big", "no", "any", "every", "each", "only", "right", "wrong", "same", "other", "number",
    "have", "want", "need", "got", "better", "best", "small", "old",
];
/// After a conjunction, "then"/"now" open the next clause instead of closing this one.
const CLAUSE_JOINERS: &[&str] = &["and", "but", "so", "or", "because"];
/// Before a final "you"/"it" these make it the subject of a clause still to come.
const BEFORE_SUBJECT: &[&str] = &[
    "if",
    "when",
    "but",
    "because",
    "that",
    "so",
    "which",
    "while",
    "as",
    "since",
    "unless",
    "until",
    "whether",
    "then",
    "before",
    "once",
    "what",
    "how",
    "why",
    "where",
    "who",
    "think",
    "guess",
    "hope",
    "believe",
    "suppose",
    "sure",
    "wish",
    "bet",
    "thought",
    "assume",
    "mean",
    "means",
    "maybe",
    "perhaps",
    "hopefully",
    "well",
    "okay",
    "now",
    "though",
    "although",
    "whenever",
    "whatever",
];
/// Verbs that want a second object after "me"/"them": "tell me" (what?).
const DITRANSITIVE: &[&str] = &[
    "tell", "give", "let", "lets", "remind", "bring", "get", "make",
];
const HER_HOSTS: &[&str] = &[
    "tell", "told", "ask", "asked", "call", "called", "love", "like", "with", "to", "for", "about",
    "see", "saw", "know", "met", "meet", "help", "from",
];

const NUMBERS: &[&str] = &[
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
    "twenty",
    "thirty",
    "forty",
    "fifty",
    "sixty",
    "seventy",
    "eighty",
    "ninety",
    "hundred",
    "thousand",
    "million",
    "billion",
];
/// After these a number is the whole answer rather than the start of a quantity.
const NUMBER_HOSTS: &[&str] = &[
    "plus", "minus", "times", "by", "number", "channel", "page", "chapter", "level", "version",
    "step", "option", "line", "at", "is", "equals", "to", "room", "season", "episode",
];
const ORDINALS: &[&str] = &[
    "first", "second", "third", "fourth", "fifth", "last", "next", "previous", "final", "other",
];
const DETERMINERS: &[&str] = &[
    "the", "a", "an", "my", "your", "our", "their", "his", "her", "its", "this", "that", "every",
    "each", "any", "some", "one", "another", "no",
];
/// The determiners that cannot stand alone, so an adjective after one is mid-phrase. ("Is
/// that right?" ends on its adjective; "the right" does not.)
const ARTICLES: &[&str] = &[
    "the", "a", "an", "my", "your", "our", "their", "his", "its", "every", "another",
];
/// Common adjectives that, straight after such a determiner, are waiting for their noun.
const ATTRIBUTIVE: &[&str] = &[
    "big",
    "small",
    "new",
    "old",
    "good",
    "bad",
    "great",
    "long",
    "short",
    "different",
    "whole",
    "certain",
    "specific",
    "particular",
    "real",
    "quick",
    "simple",
    "nice",
    "best",
    "better",
    "main",
    "large",
    "high",
    "low",
    "free",
    "entire",
    "single",
    "right",
    "left",
    "wrong",
    "nearest",
    "closest",
    "latest",
    "current",
];
const OPEN_QUANTIFIERS: &[&str] = &[
    "some", "any", "both", "each", "another", "few", "lots", "most", "less", "least", "little",
    "couple", "no", "etc", "whatever",
];
/// Adverbs that can close a sentence or open the rest of it.
const SENTENCE_ADVERBS: &[&str] = &[
    "really",
    "pretty",
    "also",
    "still",
    "maybe",
    "probably",
    "actually",
    "basically",
    "literally",
    "honestly",
    "obviously",
    "apparently",
    "perhaps",
    "usually",
    "always",
    "never",
    "often",
    "sometimes",
    "anyway",
    "anyways",
];

/// Verbs that are almost never the last word unless their object was fronted.
const NEEDS_OBJECT: &[&str] = &[
    "want",
    "wants",
    "wanted",
    "need",
    "needs",
    "needed",
    "get",
    "got",
    "getting",
    "make",
    "making",
    "put",
    "putting",
    "take",
    "taking",
    "took",
    "give",
    "giving",
    "gave",
    "tell",
    "telling",
    "show",
    "showing",
    "let",
    "set",
    "setting",
    "find",
    "finding",
    "use",
    "using",
    "send",
    "sending",
    "bring",
    "bringing",
    "add",
    "adding",
    "love",
    "hate",
    "prefer",
    "enjoy",
    "called",
    "named",
    "buy",
    "create",
    "build",
    "remove",
    "delete",
    "include",
    "contain",
    "require",
    "mention",
    "discuss",
    "describe",
    "become",
    "became",
    "keeps",
    "kept",
    "pick",
    "choose",
    "remind",
    "turn",
    "define",
    "translate",
    "summarize",
    "summarise",
    "compare",
    "convert",
    "install",
    "enable",
    "disable",
    "ask",
    "having",
];
/// Verbs that often take an object but can end a sentence: "let me explain".
const SOFT_TRANSITIVE: &[&str] = &[
    "read", "open", "play", "call", "check", "write", "save", "copy", "paste", "type", "repeat",
    "explain", "count", "order", "fix", "print", "share", "select", "switch", "change", "update",
    "move", "close", "try", "trying", "looking",
];
/// Verbs that usually introduce a clause: "I was wondering" (whether ...).
const STRONG_COMPLEMENT: &[&str] = &[
    "wondering",
    "wonder",
    "thinking",
    "means",
    "seems",
    "figured",
    "assume",
    "suppose",
    "bet",
    "wish",
    "feel",
    "felt",
    "believe",
    "hope",
    "hoping",
    "heard",
    "told",
    "thought",
    "saying",
    "says",
];
const WEAK_COMPLEMENT: &[&str] = &[
    "think", "say", "said", "guess", "know", "see", "mean", "remember",
];

/// Verbs that open a command.
const IMPERATIVES: &[&str] = &[
    "read",
    "set",
    "tell",
    "show",
    "open",
    "close",
    "play",
    "stop",
    "start",
    "turn",
    "give",
    "send",
    "call",
    "make",
    "find",
    "search",
    "look",
    "check",
    "remind",
    "add",
    "remove",
    "delete",
    "write",
    "create",
    "go",
    "take",
    "put",
    "save",
    "copy",
    "paste",
    "type",
    "say",
    "repeat",
    "explain",
    "describe",
    "summarize",
    "summarise",
    "translate",
    "define",
    "list",
    "count",
    "help",
    "let",
    "try",
    "run",
    "cancel",
    "pause",
    "resume",
    "skip",
    "scroll",
    "click",
    "select",
    "switch",
    "move",
    "change",
    "update",
    "schedule",
    "book",
    "order",
    "buy",
    "get",
    "keep",
    "continue",
    "wake",
    "mute",
    "unmute",
    "pick",
    "choose",
    "compare",
    "calculate",
    "convert",
    "spell",
    "name",
    "sing",
    "wait",
    "forget",
    "ignore",
    "remember",
    "note",
    "use",
    "come",
    "bring",
    "clear",
    "reset",
    "restart",
    "launch",
    "quit",
    "exit",
    "enable",
    "disable",
    "increase",
    "decrease",
    "lower",
    "raise",
    "zoom",
    "shut",
    "answer",
    "ask",
    "imagine",
    "draft",
    "rewrite",
    "fix",
    "format",
    "sort",
    "print",
    "share",
    "download",
    "upload",
    "install",
    "text",
    "email",
    "message",
    "thank",
    "lets",
];
/// Commands with these verbs are unfinished until they say where to: "turn the volume" (down).
const NEEDS_DIRECTION: &[&str] = &["turn", "put", "switch"];
const DIRECTIONS: &[&str] = &[
    "up", "down", "on", "off", "in", "to", "into", "at", "for", "over", "around", "back", "out",
    "away", "left", "right", "here", "there", "onto", "under", "through",
];
/// Further verb forms that show a statement has a verb.
const OTHER_VERBS: &[&str] = &[
    "went", "said", "got", "came", "took", "saw", "knew", "thought", "told", "felt", "made",
    "gave", "found", "left", "kept", "ran", "know", "think", "like", "love", "want", "need",
    "mean", "feel", "seem", "sound", "depend", "work", "agree", "hope", "guess", "see", "hear",
    "live", "happen", "matter",
];
const ABBREVIATIONS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "vs", "e.g", "i.e",
];

fn tokenize(transcript: &str) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    for raw in transcript.split_whitespace() {
        let raw = raw.replace(['’', '‘'], "'");
        let text = raw
            .trim_matches(|character: char| !character.is_alphanumeric())
            .to_lowercase();
        let after = raw
            .char_indices()
            .rev()
            .find(|(_, character)| character.is_alphanumeric())
            .map_or(raw.as_str(), |(index, character)| {
                &raw[index + character.len_utf8()..]
            });
        let comma = after.contains([',', ';', ':']);
        let stop = after.contains(['.', '?', '!']);
        if text.is_empty() {
            // Punctuation standing alone belongs to the word before it.
            if let Some(last) = tokens.last_mut() {
                last.comma |= comma;
                last.stop |= stop;
            }
            continue;
        }
        let initial = text.chars().count() == 1
            && text.chars().all(char::is_alphabetic)
            && text != "i"
            && text != "a";
        tokens.push(Token {
            stop: stop && !initial && !ABBREVIATIONS.contains(&text.as_str()),
            comma,
            text,
        });
    }
    tokens
}

fn ending_of(transcript: &str) -> Ending {
    let trimmed = transcript.trim_end();
    let cut = trimmed
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_alphanumeric())
        .map_or(0, |(index, character)| index + character.len_utf8());
    let marks = &trimmed[cut..];
    if marks.contains([',', ';', ':', '-', '–', '—', '…']) || marks.contains("..") {
        Ending::Continuing
    } else if marks.contains('?') {
        Ending::Question
    } else if marks.contains(['.', '!']) {
        Ending::Stop
    } else {
        Ending::None
    }
}

/// An opened quote or bracket that was never closed. Single quotes are skipped because they
/// cannot be told from apostrophes.
fn has_unclosed_pair(transcript: &str) -> bool {
    let count = |wanted: char| {
        transcript
            .chars()
            .filter(|character| *character == wanted)
            .count()
    };
    count('"') % 2 == 1
        || count('“') > count('”')
        || count('(') > count(')')
        || count('[') > count(']')
}

/// Drop immediate repeats ("the the", "we we"): a stumble, not content.
fn without_repeats(tokens: &[Token]) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    for token in tokens {
        match out.last_mut() {
            Some(last) if last.text == token.text => {
                last.comma |= token.comma;
                last.stop |= token.stop;
            }
            _ => out.push(token.clone()),
        }
    }
    out
}

/// Whether the words are nothing but conventional utterances: "okay, that's all, thanks".
fn is_conventional(words: &[&str], marked: bool) -> bool {
    let words: Vec<String> = words
        .iter()
        .filter(|word| **word != "please")
        .skip_while(|word| FILLERS.contains(word) || **word == "oh")
        .map(|word| word.replace('\'', ""))
        .collect();
    if words.is_empty() {
        return false;
    }
    let mut position = 0;
    while position < words.len() {
        let longest = MAX_CONVENTIONAL_WORDS.min(words.len() - position);
        let matched = (1..=longest).rev().find(|length| {
            let phrase = words[position..position + length].join(" ");
            CONVENTIONAL.contains(&phrase.as_str())
                || (marked && CONVENTIONAL_IF_MARKED.contains(&phrase.as_str()))
        });
        match matched {
            Some(length) => position += length,
            None => return false,
        }
    }
    true
}

/// Whether the speaker stopped on a hesitation: they are holding the floor, not yielding it.
fn ends_on_filler(words: &[&str]) -> bool {
    let count = words.len();
    let last = words[count - 1];
    let previous = if count >= 2 { words[count - 2] } else { "" };
    let before = if count >= 3 { words[count - 3] } else { "" };

    if FILLERS.contains(&last) {
        return true;
    }
    if matches!(last, "well" | "wait") && count <= 2 {
        return true;
    }
    match (previous, last) {
        ("hold", "on") | ("hang", "on") | ("let's", "see") | ("lets", "see") => true,
        // "Do you know" and "as you know" use the words; "it's like, you know" does not.
        ("you", "know") => !matches!(
            before,
            "do" | "did"
                | "don't"
                | "didn't"
                | "as"
                | "if"
                | "what"
                | "how"
                | "would"
                | "should"
                | "could"
                | "can"
                | "when"
                | "because"
                | "that"
                | "let"
                | "think"
                | "and"
                | "sure"
        ),
        ("i", "mean") => !matches!(before, "what" | "that" | "how" | "know"),
        ("a" | "one", "second" | "sec" | "moment" | "minute") => count <= 5,
        ("me", "think" | "see" | "check") => before == "let",
        _ => false,
    }
}

/// How many opening words are lead-ins with no content: "um, so, okay".
fn lead_in_len(words: &[&str]) -> usize {
    let mut index = 0;
    while index < words.len() {
        let word = words[index];
        let next = words.get(index + 1).copied().unwrap_or("");
        if word == "no" && AFTER_DETERMINER_NO.contains(&next) {
            break;
        }
        if FILLERS.contains(&word)
            || HESITANT_LEADS.contains(&word)
            || NEUTRAL_LEADS.contains(&word)
        {
            index += 1;
        } else if word == "i" && next == "mean" && index + 2 < words.len() {
            index += 2;
        } else {
            break;
        }
    }
    index
}

fn base_of(word: &str) -> &str {
    word.split('\'').next().unwrap_or(word)
}

fn is_negative_aux(word: &str) -> bool {
    word.ends_with("n't") || word == "cannot"
}

fn is_be(word: &str) -> bool {
    BE_FORMS.contains(&word)
}

fn is_aux(word: &str) -> bool {
    is_be(word)
        || MODALS.contains(&word)
        || OTHER_AUXILIARIES.contains(&word)
        || is_negative_aux(word)
}

/// A pronoun or question word with its auxiliary attached ("I'm", "what's"), or a possessive.
fn is_contraction(word: &str) -> bool {
    ["'s", "'re", "'m", "'ll", "'d", "'ve"]
        .iter()
        .any(|suffix| word.ends_with(suffix))
}

fn first_aux_index(words: &[&str]) -> Option<usize> {
    words
        .iter()
        .position(|word| is_aux(word) || is_contraction(word))
}

fn has_negation(words: &[&str]) -> bool {
    words
        .iter()
        .any(|word| is_negative_aux(word) || matches!(*word, "not" | "never" | "no" | "nor"))
}

fn is_number(word: &str) -> bool {
    NUMBERS.contains(&word)
        || word
            .chars()
            .all(|character| character.is_ascii_digit() || matches!(character, '.' | ',' | '%'))
}

/// Whether `word` ends in "-ing" as a verb form, not as "thing" or "something".
fn is_ing_verb(word: &str) -> bool {
    word.len() > 4 && word.ends_with("ing") && !word.ends_with("thing")
}

fn is_known_verb(word: &str) -> bool {
    let stem = word.strip_suffix('s').unwrap_or(word);
    IMPERATIVES.contains(&word)
        || IMPERATIVES.contains(&stem)
        || OTHER_VERBS.contains(&word)
        || OTHER_VERBS.contains(&stem)
}

fn has_verb(words: &[&str]) -> bool {
    words.iter().enumerate().any(|(index, word)| {
        is_aux(word)
            || is_contraction(word)
            || is_known_verb(word)
            || (word.len() > 4 && word.ends_with("ed"))
            || (index > 0 && PRONOUNISH[..7].contains(&words[index - 1]))
    })
}

/// Whether a clause that opened with "if"/"when" is followed, after a comma, by more.
fn has_main_clause(tokens: &[Token]) -> bool {
    tokens
        .iter()
        .rposition(|token| token.comma)
        .is_some_and(|index| tokens.len() - index > 2)
}

fn shape_of(words: &[&str]) -> Shape {
    let first = words[0];
    let second = words.get(1).copied().unwrap_or("");

    if first == "let's" {
        return Shape::Imperative;
    }
    if WH_WORDS.contains(&base_of(first)) {
        if first.contains('\'') {
            return Shape::WhQuestion;
        }
        let question = is_aux(second)
            || (first == "why" && second == "not")
            || (first == "how" && HOW_COMPANIONS.contains(&second))
            || (matches!(first, "what" | "who" | "which" | "whose" | "whom")
                && !second.is_empty()
                && second != "of"
                && !CLAUSE_SUBJECTS.contains(&second));
        return if question {
            Shape::WhQuestion
        } else if matches!(first, "what" | "who" | "which" | "whose" | "whom") {
            // "What I want is ..." is a statement.
            Shape::Declarative
        } else {
            Shape::Subordinate
        };
    }
    if is_aux(first) && !second.is_empty() {
        // "Do it now", "have a look" and "don't touch that" are commands; "do you",
        // "have they" and "don't we" are questions.
        let asks = matches!(second, "you" | "we" | "they" | "i");
        return match first {
            "do" | "don't" | "have" if !asks => Shape::Imperative,
            "be" | "being" | "been" => Shape::Imperative,
            _ => Shape::Inversion,
        };
    }
    if SUBORDINATORS.contains(&first) {
        return Shape::Subordinate;
    }
    if IMPERATIVES.contains(&first) {
        return Shape::Imperative;
    }
    let request = (first == "i" && matches!(second, "want" | "need" | "wanna"))
        || (first == "i'd" && second == "like")
        || (first == "i" && second == "would" && words.get(2) == Some(&"like"));
    if request {
        Shape::Request
    } else {
        Shape::Declarative
    }
}

/// Decide what the final word says about the sentence. Most of the work is telling apart the
/// two lives of words such as "that", "to", "on", "so" and "like".
fn tail_of(words: &[&str], shape: Shape, ending: Ending, answer_lead: bool) -> Tail {
    use Tail::{Closed, Dangling, Open};

    let count = words.len();
    let last = words[count - 1];
    let previous = if count >= 2 { words[count - 2] } else { "" };
    let before = if count >= 3 { words[count - 3] } else { "" };
    let marked = ending != Ending::None;
    let asked = ending == Ending::Question;
    let question = matches!(shape, Shape::WhQuestion | Shape::Inversion);
    let wh_question = shape == Shape::WhQuestion;
    let earlier = &words[..count - 1];

    // The thing the final verb or preposition needs may have been said first: "what do you
    // want", "that's all I need".
    // A question about what something is fronts no object: "what's the best way to get"
    // still needs one.
    let copular = if words[0].contains('\'') {
        !PRONOUNISH.contains(&words.get(1).copied().unwrap_or(""))
    } else {
        is_be(words.get(1).copied().unwrap_or(""))
            && ARTICLES.contains(&words.get(2).copied().unwrap_or(""))
    };
    let object_fronted = (wh_question
        && !copular
        && (OBJECT_WH.contains(&base_of(words[0]))
            || (words[0] == "how" && matches!(words.get(1), Some(&"many") | Some(&"much")))))
        || earlier
            .iter()
            .skip(1)
            .any(|word| FRONTED_HINTS.contains(&base_of(word)));
    let by_marking = if marked { Open } else { Dangling };
    // "I do." / "No, it isn't." answer a question; "I would" and "why don't" lead on.
    let short_answer = || {
        if count == 2 && PRONOUNISH.contains(&previous) {
            if answer_lead {
                Closed
            } else {
                by_marking
            }
        } else if asked && count <= 3 {
            Open
        } else {
            Dangling
        }
    };
    let hosted = |host: &str| {
        OBJECT_WORDS.contains(&host) || is_ing_verb(host) || STRAND_HOSTS.contains(&host)
    };

    if is_negative_aux(last) {
        return short_answer();
    }
    if is_contraction(last) {
        return Dangling;
    }

    match last {
        "to" => {
            if INFINITIVE_TAKERS.contains(&previous) && !OBJECT_WORDS.contains(&previous) {
                // "Do you want to?" can stand; "I want to" cannot.
                if shape == Shape::Inversion && asked {
                    Open
                } else {
                    Dangling
                }
            } else if (question || object_fronted) && hosted(previous) {
                Closed
            } else {
                Dangling
            }
        }
        "of" => {
            if question && OF_HOSTS.contains(&previous) {
                Closed
            } else {
                Dangling
            }
        }
        "in" | "on" => {
            // "Is the TV on?" asks about a state; the question mark is what tells it from
            // "is the book on" (the table).
            let state_question = shape == Shape::Inversion && is_be(words[0]) && asked;
            // Closed after an object or a host word, or for a state question.
            if OBJECT_WORDS.contains(&previous)
                || IN_ON_HOSTS.contains(&previous)
                || state_question
                || (question && hosted(previous))
            {
                Closed
            } else {
                by_marking
            }
        }
        "by" => {
            if BY_HOSTS.contains(&previous) {
                Closed
            } else {
                Dangling
            }
        }
        "like" => {
            // "What's it like?" against "I was like".
            if wh_question {
                Closed
            } else if previous == "you" && earlier.iter().any(|word| matches!(*word, "if" | "as")) {
                Open
            } else {
                Dangling
            }
        }
        "that" | "this" | "these" | "those" => {
            let told = OBJECT_WORDS.contains(&previous) && TELL_VERBS.contains(&before);
            if last == "that" && (THAT_TAKERS.contains(&previous) || told) {
                // "Who said that?" and "What is that" point at something; "I think that"
                // introduces a clause.
                if (question && asked) || (wh_question && is_be(previous) && count == 3) {
                    Closed
                } else {
                    by_marking
                }
            } else if last != "that" {
                // "This" is rarely anything but an object when it comes last, unless the
                // word before it was about to start a clause: "I think this".
                if BEFORE_SUBJECT.contains(&previous) {
                    Dangling
                } else if count == 1
                    || CLAUSE_JOINERS.contains(&previous)
                    || (THAT_TAKERS.contains(&previous) && !is_be(previous))
                {
                    Open
                } else {
                    Closed
                }
            } else if DEMONSTRATIVE_HOSTS.contains(&previous)
                || is_ing_verb(previous)
                || (previous.len() > 4 && previous.ends_with("ed"))
                || (shape == Shape::Imperative && count == 2)
            {
                Closed
            } else {
                // "The book that" opens a relative clause.
                by_marking
            }
        }
        "so" => {
            // "I think so", "I told you so".
            if SO_HOSTS.contains(&previous) || (previous == "you" && before == "told") {
                Closed
            } else {
                Dangling
            }
        }
        "not" => {
            if NOT_HOSTS.contains(&previous) {
                Closed
            } else {
                Dangling
            }
        }
        "then" | "now" => {
            if CLAUSE_JOINERS.contains(&previous) || (last == "then" && previous == "if") {
                Dangling
            } else {
                Closed
            }
        }
        "either" => {
            if has_negation(earlier) {
                Closed
            } else {
                Dangling
            }
        }
        "too" => {
            // "Me too" against "it is too" (big).
            if count < 2
                || is_aux(previous)
                || is_contraction(previous)
                || matches!(previous, "and" | "but" | "not" | "way" | "far")
            {
                Dangling
            } else {
                Closed
            }
        }
        "though" => {
            if matches!(previous, "even" | "as") {
                Dangling
            } else if count >= 3 {
                Closed
            } else {
                Open
            }
        }
        "all" => {
            if matches!(
                previous,
                "at" | "them" | "it" | "us" | "you" | "that" | "after" | "of"
            ) {
                Closed
            } else {
                Open
            }
        }
        "more" => {
            if MORE_HOSTS.contains(&previous) {
                Closed
            } else {
                Open
            }
        }
        "much" | "many" => match previous {
            "as" => Dangling,
            "how" | "too" | "so" if last == "many" => Open,
            "how" => Open,
            _ if last == "much" => Closed,
            _ => Open,
        },
        "there" => {
            if wh_question && count == 3 {
                Closed
            } else if is_be(previous)
                || is_negative_aux(previous)
                || matches!(
                    previous,
                    "if" | "and" | "but" | "that" | "because" | "so" | "when"
                )
            {
                Dangling
            } else {
                Closed
            }
        }
        "her" => {
            if HER_HOSTS.contains(&previous) {
                Closed
            } else {
                Open
            }
        }
        "me" | "him" | "them" | "us" => {
            if DITRANSITIVE.contains(&previous) {
                Dangling
            } else {
                Closed
            }
        }
        "i" | "we" | "they" | "he" | "she" | "you" | "it" => {
            let object_capable = matches!(last, "you" | "it");
            if is_aux(previous) || is_contraction(previous) {
                // "..., isn't it?" and "you don't mind, do you?"
                let tag = count >= 4
                    && !question
                    && (is_negative_aux(previous) || has_negation(&words[..count - 2]));
                // "Where am I", "what time is it": "be" is the main verb.
                let be_question = wh_question
                    && base_of(words[0]) != "why"
                    && is_be(previous)
                    && first_aux_index(words) == Some(count - 2);
                let inverted = question && first_aux_index(words) == Some(count - 2);
                if tag || be_question {
                    Closed
                } else if shape == Shape::Inversion && count == 2 {
                    // "Can you" opens a request; "Is it?" may be a whole reply.
                    if asked {
                        Open
                    } else {
                        Dangling
                    }
                } else if object_capable
                    && !inverted
                    && matches!(
                        previous,
                        "do" | "did" | "does" | "have" | "had" | "has" | "is" | "was"
                    )
                {
                    // "I did it", "can you do it", "this is it".
                    Closed
                } else {
                    Dangling
                }
            } else if !object_capable || BEFORE_SUBJECT.contains(&previous) {
                Dangling
            } else if matches!(previous, "and" | "or") {
                Open
            } else {
                Closed
            }
        }
        "do" => {
            if object_fronted || previous == "to" {
                Closed
            } else {
                short_answer()
            }
        }
        "doing" => {
            if wh_question {
                Closed
            } else if is_aux(previous) || is_contraction(previous) {
                Dangling
            } else {
                Open
            }
        }
        "going" => {
            if wh_question || previous == "keep" {
                Closed
            } else {
                Open
            }
        }
        "have" | "be" | "been" => {
            if wh_question && (last != "have" || object_fronted) {
                // "Where have you been", "what could it be", "what do you have".
                Closed
            } else if last == "have" {
                short_answer()
            } else {
                Dangling
            }
        }
        // "What's five times" is a sum in progress; "three times" on its own may not be.
        "times" | "minus" | "divided" => {
            if is_number(previous) {
                Open
            } else if last == "times" {
                Closed
            } else {
                Dangling
            }
        }
        "one" if ONE_HOSTS.contains(&previous) => Closed,
        "other" if previous == "each" => Closed,
        "while" if previous == "a" => Closed,
        "bit" | "lot" => Closed,
        "know" if previous == "me" && before == "let" => Closed,
        "well" => {
            if count <= 2 || CLAUSE_JOINERS.contains(&previous) {
                Dangling
            } else {
                Closed
            }
        }
        "okay" | "ok" | "yeah" | "alright" => {
            if CLAUSE_JOINERS.contains(&previous) {
                Open
            } else {
                Closed
            }
        }
        _ if is_aux(last) => {
            // "Do you know where the station is": the clause after a question word may end
            // on "be". "What I want is" does not.
            let embedded = is_be(last)
                && earlier.iter().enumerate().any(|(index, word)| {
                    index >= 1
                        && WH_WORDS.contains(word)
                        && count - 1 - index >= 2
                        && !words[index..count - 1]
                            .iter()
                            .any(|between| matches!(*between, "but" | "because" | "so"))
                });
            // "What do you think it is": the same, with the question word first.
            let reported = is_be(last)
                && wh_question
                && count >= 5
                && PRONOUNISH.contains(&previous)
                && is_aux(words[1]);
            if embedded || reported {
                Closed
            } else {
                short_answer()
            }
        }
        _ if ALWAYS_DANGLING.contains(&last) => Dangling,
        _ if WH_WORDS.contains(&last) => {
            if marked && count <= 3 {
                Open
            } else {
                Dangling
            }
        }
        _ if STRANDABLE.contains(&last) => {
            if (question || object_fronted) && hosted(previous) {
                Closed
            } else {
                Dangling
            }
        }
        _ if PARTICLES.contains(&last) => {
            if PHRASAL_TRANSITIVE.contains(&previous) {
                Open
            } else {
                Closed
            }
        }
        _ if LOOSE_PARTICLES.contains(&last) => {
            if OBJECT_WORDS.contains(&previous) {
                Closed
            } else {
                Open
            }
        }
        _ if ORDINALS.contains(&last) => {
            // "The next" and a lone "first" are counting towards something.
            if count == 1 || DETERMINERS.contains(&previous) || CLAUSE_JOINERS.contains(&previous) {
                Dangling
            } else {
                Closed
            }
        }
        _ if is_number(last) => {
            if NUMBER_HOSTS.contains(&previous) {
                Closed
            } else {
                Open
            }
        }
        _ if ATTRIBUTIVE.contains(&last) && ARTICLES.contains(&previous) => Dangling,
        _ if OPEN_QUANTIFIERS.contains(&last) => Open,
        _ if SENTENCE_ADVERBS.contains(&last) => {
            if count == 1
                || SUBJECT_PRONOUNS.contains(&previous)
                || is_aux(previous)
                || is_contraction(previous)
                || CLAUSE_JOINERS.contains(&previous)
            {
                Dangling
            } else {
                Open
            }
        }
        _ if NEEDS_OBJECT.contains(&last) => {
            if object_fronted {
                Closed
            } else {
                by_marking
            }
        }
        _ if SOFT_TRANSITIVE.contains(&last) => {
            if object_fronted {
                Closed
            } else {
                Open
            }
        }
        _ if STRONG_COMPLEMENT.contains(&last) => {
            if object_fronted {
                Closed
            } else {
                by_marking
            }
        }
        _ if WEAK_COMPLEMENT.contains(&last) => {
            if object_fronted {
                Closed
            } else if !marked
                && matches!(last, "think" | "mean" | "say")
                && (SUBJECT_PRONOUNS.contains(&previous) || is_negative_aux(previous))
            {
                // "I think", "I don't think", "I mean".
                Dangling
            } else {
                Open
            }
        }
        _ if count == 1 => Open,
        _ => Closed,
    }
}

#[cfg(test)]
mod tests {
    use super::Completeness::{Complete, Incomplete, Unsure};
    use super::*;

    /// Written and labelled before the rules were tuned; the rules were then tuned on it.
    const CORPUS: &[(&str, Completeness)] = &[
        // Questions.
        ("What's the capital of France?", Complete),
        ("what is the capital of france", Complete),
        ("How do I stop a process in PowerShell?", Complete),
        ("Who wrote Pride and Prejudice?", Complete),
        ("What time is it?", Complete),
        ("what time is it", Complete),
        ("How many people live in Tokyo", Complete),
        ("Can you tell me a joke?", Complete),
        ("Could you read this page to me?", Complete),
        ("Is it going to rain tomorrow?", Complete),
        ("Do you know where the nearest station is?", Complete),
        ("Where are you from?", Complete),
        ("Who should I send it to?", Complete),
        ("What are you talking about?", Complete),
        ("What's going on?", Complete),
        ("What's it like?", Complete),
        ("What does it look like", Complete),
        ("Why is the sky blue?", Complete),
        ("When does the meeting start?", Complete),
        ("Which one should I pick?", Complete),
        ("How are you?", Complete),
        ("how are you doing today", Complete),
        ("Are you there?", Complete),
        ("Does that make sense?", Complete),
        ("What do you think?", Complete),
        ("What should I do?", Complete),
        ("What did you say?", Complete),
        ("Is that right?", Complete),
        ("That's the one, right?", Complete),
        ("It's cold today, isn't it?", Complete),
        ("What's two plus two?", Complete),
        ("How much does it cost", Complete),
        ("What is that?", Complete),
        ("Who is this for?", Complete),
        ("Can you help me with my homework?", Complete),
        ("What happened then?", Complete),
        ("Should I do it now?", Complete),
        ("Um, what's the weather like in Boston?", Complete),
        ("So how do I fix it?", Complete),
        ("Would you mind repeating that", Complete),
        ("What are you looking at?", Complete),
        ("Where did you put it?", Complete),
        ("And what about the other one?", Complete),
        ("How about pizza?", Complete),
        ("Why not?", Complete),
        ("Did you get that?", Complete),
        ("What does that mean?", Complete),
        ("Have you seen my keys", Complete),
        ("Who are you going with?", Complete),
        ("Is the TV on?", Complete),
        // Commands and requests.
        ("Read this page.", Complete),
        ("read this page", Complete),
        ("Set a timer for ten minutes.", Complete),
        ("set a timer for ten minutes", Complete),
        ("Tell me a joke.", Complete),
        ("tell me a joke", Complete),
        ("Turn it up.", Complete),
        ("Turn the volume down", Complete),
        ("Open the settings window.", Complete),
        ("Please stop.", Complete),
        ("Play some music", Complete),
        ("Remind me to call mom at five.", Complete),
        ("Go back to the previous paragraph.", Complete),
        ("Read that again, please.", Complete),
        ("Explain how photosynthesis works.", Complete),
        ("Give me a summary of this article", Complete),
        ("Let's start over.", Complete),
        ("Show me the next one.", Complete),
        ("Translate hello into Spanish.", Complete),
        ("Don't do that.", Complete),
        ("Tell me more.", Complete),
        ("Look it up.", Complete),
        ("Send it to him.", Complete),
        ("Shut it down.", Complete),
        ("Come in.", Complete),
        ("Do it now.", Complete),
        // Short conventional utterances.
        ("Yes.", Complete),
        ("no", Complete),
        ("Thanks.", Complete),
        ("Okay.", Complete),
        ("stop", Complete),
        ("Goodbye.", Complete),
        ("Never mind.", Complete),
        ("That's all.", Complete),
        ("Go on.", Complete),
        ("Yeah, thanks.", Complete),
        ("OK, that's all, thanks.", Complete),
        ("I think so.", Complete),
        ("I don't know.", Complete),
        ("Not really.", Complete),
        ("Thank you very much!", Complete),
        ("Sounds good.", Complete),
        ("Maybe later", Complete),
        ("Okay then.", Complete),
        ("Me too.", Complete),
        // Statements.
        ("I don't like that.", Complete),
        ("I don't like that", Complete),
        ("I went to the store yesterday.", Complete),
        ("My name is Sam.", Complete),
        ("That's not what I meant.", Complete),
        ("I think it's broken.", Complete),
        ("I don't like it either.", Complete),
        ("I want one too.", Complete),
        ("It was pretty good, though.", Complete),
        ("I'll do it later.", Complete),
        ("I'm looking for a good Italian restaurant.", Complete),
        ("The meeting was moved to Thursday.", Complete),
        ("I already tried that.", Complete),
        ("We can talk about it now.", Complete),
        ("That sounds like a great idea.", Complete),
        ("I was just kidding.", Complete),
        ("I need to buy some milk and eggs.", Complete),
        ("It doesn't work.", Complete),
        ("I see what you mean.", Complete),
        ("That's what I was looking for.", Complete),
        ("I have no idea.", Complete),
        ("I'd like to book a table for two.", Complete),
        ("She said she would call me back.", Complete),
        ("I'm not sure about that.", Complete),
        ("It's right there.", Complete),
        ("Something like that.", Complete),
        ("It depends.", Complete),
        // Trailing off.
        ("So what I was thinking is that we could", Incomplete),
        ("I was thinking that we could.", Incomplete),
        ("Can you", Incomplete),
        ("I want to", Incomplete),
        ("what about the", Incomplete),
        ("How do I", Incomplete),
        ("I went to the store and", Incomplete),
        ("I went to the store and.", Incomplete),
        ("It didn't work because", Incomplete),
        ("The thing is,", Incomplete),
        ("I need to buy milk, eggs,", Incomplete),
        ("one, two,", Incomplete),
        ("First", Incomplete),
        ("um", Incomplete),
        ("So, um,", Incomplete),
        ("I mean", Incomplete),
        ("Let me think", Incomplete),
        ("Hold on.", Incomplete),
        ("It's like, you know", Incomplete),
        ("What's the capital of", Incomplete),
        ("Tell me about the", Incomplete),
        ("I think that", Incomplete),
        ("I would", Incomplete),
        ("The problem is", Incomplete),
        ("Could you please", Incomplete),
        ("I'm looking for a", Incomplete),
        ("If you go to the settings and then", Incomplete),
        ("Well, I was wondering if", Incomplete),
        ("And then she said \"I don't", Incomplete),
        ("The function (the one I wrote", Incomplete),
        ("I was going to say...", Incomplete),
        ("But what I really want is —", Incomplete),
        ("My", Incomplete),
        ("the", Incomplete),
        ("so I", Incomplete),
        ("When I get home", Incomplete),
        ("If it rains tomorrow", Incomplete),
        ("What I mean is", Incomplete),
        ("I want you to", Incomplete),
        ("Set a timer for", Incomplete),
        ("Can you tell me", Incomplete),
        ("Is there a", Incomplete),
        ("It's kind of", Incomplete),
        ("Which is", Incomplete),
        ("There are three things: first,", Incomplete),
        ("What's the difference between", Incomplete),
        ("I'd like to know whether", Incomplete),
        ("Remind me to", Incomplete),
        ("So basically", Incomplete),
        ("I don't know if", Incomplete),
        ("That's because the", Incomplete),
        ("Do you think we should", Incomplete),
        ("Read me the", Incomplete),
        ("What's the capital of France, um", Incomplete),
        ("I was like", Incomplete),
        ("and uh", Incomplete),
        ("I'm trying to figure out how to", Incomplete),
        ("Let me see", Incomplete),
        ("the the", Incomplete),
        ("I think we we should", Incomplete),
        ("More than", Incomplete),
        ("As far as I", Incomplete),
        ("Either we go or", Incomplete),
        ("The book that", Incomplete),
        ("And so", Incomplete),
        ("I'm working on", Incomplete),
        ("And now", Incomplete),
        ("and then", Incomplete),
        ("It's too", Incomplete),
        ("even though", Incomplete),
        ("It looks like", Incomplete),
        // Could be either.
        ("", Unsure),
        ("...", Unsure),
        ("?!", Unsure),
        ("I think.", Unsure),
        ("I do.", Unsure),
        ("The store.", Unsure),
        ("Tomorrow", Unsure),
        ("Because it's broken", Unsure),
        ("He told me that.", Unsure),
        ("The red one", Unsure),
        ("Is it?", Unsure),
        ("I guess", Unsure),
        ("Do you know", Unsure),
        ("Turn off", Unsure),
        ("In the kitchen.", Unsure),
        ("Pizza", Unsure),
        ("John", Unsure),
        ("I was thinking.", Unsure),
        ("We could go", Unsure),
        ("twenty five", Unsure),
        ("I said that", Unsure),
        ("I'd love to.", Unsure),
    ];

    /// Written and labelled with the corpus above, and never used to adjust a rule: it was
    /// first scored only after the rules were final.
    const HELD_OUT: &[(&str, Completeness)] = &[
        ("What's the tallest mountain in the world?", Complete),
        ("how old is the universe", Complete),
        ("Can you turn the lights off?", Complete),
        ("Where should I put this?", Complete),
        ("Who was the first president of the United States", Complete),
        ("Do I need an umbrella today?", Complete),
        ("What are you waiting for?", Complete),
        ("How long does it take to boil an egg?", Complete),
        ("Is this the right one?", Complete),
        ("Why did it stop working", Complete),
        ("Summarize this document for me.", Complete),
        ("Close the window", Complete),
        ("Add milk to my shopping list.", Complete),
        ("Wake me up at seven.", Complete),
        ("Keep going.", Complete),
        ("Scroll down a little.", Complete),
        ("Tell her I'm running late.", Complete),
        ("Write an email to my boss", Complete),
        ("Yep.", Complete),
        ("No thanks.", Complete),
        ("That's enough.", Complete),
        ("All right, bye.", Complete),
        ("I'm not sure.", Complete),
        ("I just wanted to say thank you.", Complete),
        ("It was raining when I left the house.", Complete),
        ("I can't find my phone.", Complete),
        ("That's exactly what I was afraid of.", Complete),
        ("You were right about that.", Complete),
        ("I'll think about it.", Complete),
        ("We should probably leave now.", Complete),
        ("I was hoping you could", Incomplete),
        ("What's the name of the", Incomplete),
        ("Can I get a", Incomplete),
        ("So the reason I'm asking is because", Incomplete),
        ("I'd like to order a pizza with", Incomplete),
        ("It's on the, uh", Incomplete),
        ("Do you know how to", Incomplete),
        ("Tell me what", Incomplete),
        ("My favorite color is", Incomplete),
        ("I need you to send an email to John and", Incomplete),
        ("Well, you know,", Incomplete),
        ("Hang on a second", Incomplete),
        ("The other day I was", Incomplete),
        ("What if we", Incomplete),
        ("Not only that, but", Incomplete),
        ("He said \"wait", Incomplete),
        ("Search for", Incomplete),
        ("Sometimes.", Unsure),
        ("The blue", Unsure),
        ("I suppose", Unsure),
        ("After lunch", Unsure),
        ("You can.", Unsure),
    ];

    struct Scores {
        complete_precision: f64,
        complete_recall: f64,
        incomplete_precision: f64,
        incomplete_recall: f64,
        /// Utterances judged `Complete` that were labelled `Incomplete`: the worst mistake.
        cut_offs: Vec<String>,
        mistakes: Vec<String>,
    }

    fn score(corpus: &[(&str, Completeness)]) -> Scores {
        let ratio = |hits: usize, total: usize| {
            if total == 0 {
                1.0
            } else {
                hits as f64 / total as f64
            }
        };
        let judged: Vec<(&str, Completeness, Completeness)> = corpus
            .iter()
            .map(|(text, expected)| (*text, *expected, judge_transcript(text)))
            .collect();
        let count = |expected: Option<Completeness>, actual: Option<Completeness>| {
            judged
                .iter()
                .filter(|(_, wanted, got)| {
                    expected.is_none_or(|label| label == *wanted)
                        && actual.is_none_or(|label| label == *got)
                })
                .count()
        };
        Scores {
            complete_precision: ratio(
                count(Some(Complete), Some(Complete)),
                count(None, Some(Complete)),
            ),
            complete_recall: ratio(
                count(Some(Complete), Some(Complete)),
                count(Some(Complete), None),
            ),
            incomplete_precision: ratio(
                count(Some(Incomplete), Some(Incomplete)),
                count(None, Some(Incomplete)),
            ),
            incomplete_recall: ratio(
                count(Some(Incomplete), Some(Incomplete)),
                count(Some(Incomplete), None),
            ),
            cut_offs: judged
                .iter()
                .filter(|(_, wanted, got)| *wanted == Incomplete && *got == Complete)
                .map(|(text, _, _)| text.to_string())
                .collect(),
            mistakes: judged
                .iter()
                .filter(|(_, wanted, got)| wanted != got)
                .map(|(text, wanted, got)| format!("{text:?}: wanted {wanted:?}, got {got:?}"))
                .collect(),
        }
    }

    fn report(name: &str, size: usize, scores: &Scores) -> String {
        format!(
            "{name} ({size} utterances): Complete precision {:.3} recall {:.3}; \
             Incomplete precision {:.3} recall {:.3}\n  {}",
            scores.complete_precision,
            scores.complete_recall,
            scores.incomplete_precision,
            scores.incomplete_recall,
            scores.mistakes.join("\n  ")
        )
    }

    fn assert_all(expected: Completeness, utterances: &[&str]) {
        for utterance in utterances {
            assert_eq!(judge_transcript(utterance), expected, "{utterance:?}");
        }
    }

    #[test]
    fn the_corpus_is_large_enough_to_mean_something() {
        assert!(CORPUS.len() >= 150, "{}", CORPUS.len());
        assert!(HELD_OUT.len() >= 50, "{}", HELD_OUT.len());
    }

    #[test]
    fn complete_and_incomplete_are_precise_on_the_corpus() {
        let scores = score(CORPUS);
        let summary = report("corpus", CORPUS.len(), &scores);
        println!("{summary}");

        assert!(scores.complete_precision >= 0.97, "{summary}");
        assert!(scores.incomplete_precision >= 0.9, "{summary}");
        // Recall is reported, with loose floors so that a rule change that guts it is seen.
        assert!(scores.complete_recall >= 0.8, "{summary}");
        assert!(scores.incomplete_recall >= 0.8, "{summary}");
        assert!(scores.cut_offs.is_empty(), "{:?}", scores.cut_offs);
    }

    #[test]
    fn held_out_utterances_are_never_cut_off() {
        let scores = score(HELD_OUT);
        let summary = report("held out", HELD_OUT.len(), &scores);
        println!("{summary}");

        // Floors chosen before the first run; fifty utterances cannot support tighter ones.
        assert!(scores.complete_precision >= 0.9, "{summary}");
        assert!(scores.incomplete_precision >= 0.85, "{summary}");
        assert!(scores.cut_offs.is_empty(), "{:?}", scores.cut_offs);
    }

    #[test]
    fn silence_and_bare_punctuation_are_unsure() {
        assert_all(Unsure, &["", "   ", "...", "?!", "\"\"", " . "]);
    }

    #[test]
    fn a_well_formed_question_is_complete_without_a_question_mark() {
        assert_all(
            Complete,
            &[
                "what is the capital of france",
                "What's the capital of France?",
                "how do i open the settings",
                "who wrote this",
                "which one is cheaper",
                "can you read this page to me",
                "is it going to rain tomorrow",
            ],
        );
    }

    #[test]
    fn a_command_with_its_object_is_complete() {
        assert_all(
            Complete,
            &[
                "read this page",
                "Read this.",
                "set a timer for ten minutes",
                "tell me a joke",
                "Stop talking.",
                "please open the settings window",
                "I'd like to hear the next chapter",
            ],
        );
    }

    #[test]
    fn a_command_missing_its_object_is_not_complete() {
        assert_all(
            Incomplete,
            &[
                "set a timer for",
                "tell me",
                "read me the",
                "remind me to",
                "open the",
            ],
        );
        assert_all(
            Unsure,
            &["set a timer for ten", "turn off", "Turn the volume"],
        );
    }

    #[test]
    fn conventional_utterances_are_complete_however_they_are_written() {
        assert_all(
            Complete,
            &[
                "yes",
                "No.",
                "THANKS",
                "okay",
                "Stop!",
                "goodbye",
                "never mind",
                "that's all",
                "thats all",
                "That’s all, thanks.",
                "go on",
                "Oh, okay.",
                "Yes, please.",
            ],
        );
    }

    #[test]
    fn a_lone_question_word_needs_its_question_mark() {
        assert_eq!(judge_transcript("What?"), Complete);
        assert_eq!(judge_transcript("what"), Incomplete);
        assert_eq!(judge_transcript("Why?"), Complete);
        assert_eq!(judge_transcript("I know."), Complete);
        assert_eq!(judge_transcript("I know"), Unsure);
    }

    #[test]
    fn a_statement_leans_on_the_recognisers_full_stop() {
        assert_eq!(judge_transcript("I went to the store yesterday."), Complete);
        assert_eq!(judge_transcript("I went to the store yesterday"), Unsure);
        // No verb: an answer fragment, or the start of something.
        assert_eq!(judge_transcript("The store."), Unsure);
    }

    #[test]
    fn a_full_stop_does_not_rescue_a_dangling_word() {
        assert_all(
            Incomplete,
            &[
                "I was thinking that we could.",
                "I went to the store and.",
                "So the.",
                "It's because of.",
                "I want to.",
                "What I mean is.",
                "Could you please.",
            ],
        );
    }

    #[test]
    fn function_words_at_the_end_are_incomplete() {
        assert_all(
            Incomplete,
            &[
                "we could go and",
                "it works but",
                "it failed because",
                "a piece of",
                "I put it in the",
                "that is my",
                "the problem is",
                "we would",
                "I'm",
                "it depends on whether",
                "bigger than",
                "that's",
                "it is very",
            ],
        );
    }

    #[test]
    fn fillers_and_hesitations_at_the_end_are_incomplete() {
        assert_all(
            Incomplete,
            &[
                "um",
                "so, uh",
                "What's the capital of France, um",
                "it's like, you know",
                "I mean",
                "let me think",
                "Let me see.",
                "hold on",
                "Hang on.",
                "just a second",
                "well",
                "I was like",
            ],
        );
    }

    #[test]
    fn a_comma_dash_or_ellipsis_at_the_end_is_incomplete() {
        assert_all(
            Incomplete,
            &[
                "I need milk, eggs,",
                "Okay,",
                "what I want is —",
                "I was go-",
                "I was going to say...",
                "and then…",
                "There are two options:",
            ],
        );
    }

    #[test]
    fn an_unclosed_quote_or_bracket_is_incomplete() {
        assert_all(
            Incomplete,
            &[
                "She said \"I will be there",
                "He wrote “see you soon",
                "The file (the big one",
            ],
        );
        assert_eq!(judge_transcript("She said \"I will be there.\""), Complete);
    }

    #[test]
    fn an_opening_phrase_with_nothing_after_it_is_incomplete() {
        assert_all(
            Incomplete,
            &[
                "can you",
                "Could you please",
                "I want to",
                "what about the",
                "how do I",
                "how about",
                "do you",
                "I was wondering",
                "what do you",
                "is there",
            ],
        );
    }

    #[test]
    fn a_count_or_list_in_progress_is_not_complete() {
        assert_all(Incomplete, &["one, two,", "first", "First,", "the next"]);
        assert_eq!(judge_transcript("one two three"), Unsure);
    }

    #[test]
    fn a_clause_that_opens_with_if_or_when_needs_its_main_clause() {
        assert_eq!(judge_transcript("when I get home"), Incomplete);
        assert_eq!(judge_transcript("If it rains tomorrow"), Incomplete);
        assert_eq!(judge_transcript("If it rains tomorrow."), Unsure);
        assert_eq!(judge_transcript("If it rains, I'll stay home."), Complete);
    }

    #[test]
    fn that_is_complete_as_a_pronoun_and_incomplete_as_a_complementiser() {
        assert_all(
            Complete,
            &[
                "I don't like that.",
                "Read that.",
                "What is that?",
                "Who said that?",
            ],
        );
        assert_all(
            Incomplete,
            &["I think that", "the book that", "the problem is that"],
        );
        // The stop makes "that" a possible pronoun, but only possible.
        assert_eq!(judge_transcript("I think that."), Unsure);
    }

    #[test]
    fn a_stranded_preposition_is_complete_only_in_a_question_that_fronts_its_object() {
        assert_all(
            Complete,
            &[
                "Who should I send it to?",
                "who should I send it to",
                "What are you looking for?",
                "Where are you from?",
                "What are you talking about",
            ],
        );
        assert_all(
            Incomplete,
            &[
                "I sent it to",
                "what do I need to",
                "What's the best way to",
                "what's the recipe for",
                "tell me about",
            ],
        );
    }

    #[test]
    fn particles_complete_a_phrasal_verb_whose_object_was_already_said() {
        assert_all(
            Complete,
            &[
                "Turn it up.",
                "What's going on?",
                "Come in.",
                "Shut it down.",
                "Look it up.",
            ],
        );
        assert_all(
            Incomplete,
            &["I'm working on", "I live in", "it depends on"],
        );
        assert_eq!(judge_transcript("turn off"), Unsure);
    }

    #[test]
    fn so_and_like_are_complete_only_where_they_end_a_phrase() {
        assert_all(
            Complete,
            &["I think so.", "I don't think so", "What's it like?"],
        );
        assert_all(
            Incomplete,
            &["and so", "it was so", "I was like", "it looks like"],
        );
    }

    #[test]
    fn sentence_final_adverbs_and_tags_do_not_block_completion() {
        assert_all(
            Complete,
            &[
                "It was good, though.",
                "I want one too.",
                "I don't like it either.",
                "What happened then?",
                "Should I do it now?",
                "That's the one, right?",
                "Is that okay?",
                "It's cold today, isn't it?",
            ],
        );
        assert_all(
            Incomplete,
            &[
                "and then",
                "but now",
                "it's too",
                "even though",
                "it's either",
            ],
        );
    }

    #[test]
    fn a_short_answer_is_complete_only_after_yes_or_no() {
        assert_eq!(judge_transcript("Yes, I do."), Complete);
        assert_eq!(judge_transcript("No, it isn't."), Complete);
        assert_eq!(judge_transcript("I do."), Unsure);
        assert_eq!(judge_transcript("I would"), Incomplete);
    }

    #[test]
    fn a_clause_after_a_question_word_may_end_on_be() {
        assert_all(
            Complete,
            &[
                "Do you know where the nearest station is?",
                "Tell me what time it is.",
                "What do you think it is?",
            ],
        );
        assert_all(
            Incomplete,
            &["What I want is", "what time is", "the question is"],
        );
    }

    #[test]
    fn very_short_fragments_are_never_complete() {
        for fragment in [
            "the", "so I", "my", "pizza", "John", "tomorrow", "the red", "and",
        ] {
            assert_ne!(judge_transcript(fragment), Complete, "{fragment:?}");
        }
    }

    #[test]
    fn only_the_last_sentence_is_judged() {
        assert_eq!(
            judge_transcript("I went to the store. What's the capital of France?"),
            Complete
        );
        assert_eq!(
            judge_transcript("What's the capital of France? And"),
            Incomplete
        );
        assert_eq!(judge_transcript("Read this page. Thanks."), Complete);
        // An abbreviation's stop does not start a new sentence.
        assert_eq!(
            judge_transcript("Is Dr. Lee in the office today?"),
            Complete
        );
    }

    #[test]
    fn casing_repeats_and_stray_punctuation_do_not_change_the_verdict() {
        assert_eq!(judge_transcript("WHAT TIME IS IT"), Complete);
        assert_eq!(judge_transcript("  what  time is it ? "), Complete);
        assert_eq!(judge_transcript("what what time is it"), Complete);
        assert_eq!(judge_transcript("Um, so, what time is it?"), Complete);
        assert_eq!(judge_transcript("I think we we should"), Incomplete);
        assert_eq!(judge_transcript("the the"), Incomplete);
        assert_eq!(judge_transcript("Tell me a joke, please."), Complete);
    }
}
