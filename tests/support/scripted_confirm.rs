use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::action::confirm::{Confirm, SelectOption};
use crate::error::Failure;

#[derive(Debug, Default)]
pub(crate) struct Scripted {
    texts: Mutex<VecDeque<Option<String>>>,
    answers: Mutex<VecDeque<bool>>,
    picks: Mutex<VecDeque<Option<usize>>>,
    multi: Mutex<VecDeque<Vec<usize>>>,
    shown: Mutex<Vec<String>>,
}

impl Scripted {
    pub(crate) fn new(
        texts: Vec<Option<&str>>,
        answers: Vec<bool>,
        picks: Vec<Option<usize>>,
        multi: Vec<Vec<usize>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            texts: Mutex::new(texts.into_iter().map(|t| t.map(str::to_owned)).collect()),
            answers: Mutex::new(answers.into()),
            picks: Mutex::new(picks.into()),
            multi: Mutex::new(multi.into()),
            shown: Mutex::default(),
        })
    }

    pub(crate) fn shown(&self) -> Vec<String> {
        self.shown.lock().unwrap().clone()
    }

    fn show(&self, prompt: &str, options: &[SelectOption]) {
        let mut shown = self.shown.lock().unwrap();
        shown.push(prompt.to_owned());
        shown.extend(options.iter().map(|o| match &o.description {
            Some(d) => format!("{} — {d}", o.id),
            None => o.id.clone(),
        }));
    }
}

impl Confirm for Scripted {
    fn confirm(&self, question: &str, caveat: Option<&str>) -> Result<bool, Failure> {
        self.shown.lock().unwrap().extend(
            caveat
                .map(str::to_owned)
                .into_iter()
                .chain([question.to_owned()]),
        );
        Ok(self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("an unscripted confirm"))
    }

    fn select_many(&self, prompt: &str, options: &[SelectOption]) -> Result<Vec<usize>, Failure> {
        self.show(prompt, options);
        Ok(self
            .multi
            .lock()
            .unwrap()
            .pop_front()
            .expect("an unscripted select_many"))
    }

    fn select_one(&self, prompt: &str, options: &[SelectOption]) -> Result<Option<usize>, Failure> {
        self.show(prompt, options);
        Ok(self
            .picks
            .lock()
            .unwrap()
            .pop_front()
            .expect("an unscripted select_one"))
    }

    fn ask_text(&self, prompt: &str, _secret: bool) -> Result<Option<String>, Failure> {
        self.shown.lock().unwrap().push(prompt.to_owned());
        Ok(self
            .texts
            .lock()
            .unwrap()
            .pop_front()
            .expect("an unscripted ask_text"))
    }

    fn is_interactive(&self) -> bool {
        true
    }
}
