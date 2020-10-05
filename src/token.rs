use std::ops::Deref;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Acquire;
use std::sync::atomic::Ordering::Release;
use std::sync::Arc;

struct AtomicFlag(AtomicBool);

impl AtomicFlag {
    pub fn new(value: bool) -> Self {
        AtomicFlag(AtomicBool::new(value))
    }

    pub fn unset() -> Self {
        Self::new(false)
    }

    pub fn set(&self) {
        self.0.store(true, Release)
    }

    pub fn reset(&self) {
        self.0.store(false, Release)
    }

    pub fn is_set(&self) -> bool {
        self.0.load(Acquire)
    }
}

pub trait CommonToken {
    type Value;

    fn value(&self) -> &Self::Value;

    fn reset(&self);
}

pub trait Completable {
    fn complete(&self);

    fn is_completed(&self) -> bool;
}

pub trait Cancelable {
    fn cancel(&self);

    fn is_canceled(&self) -> bool;
}

pub struct ValueToken<Value> {
    value: Value,
}

impl<Value> ValueToken<Value> {
    pub fn new(value: Value) -> Self {
        ValueToken { value }
    }
}

impl<Value: Default> Default for ValueToken<Value> {
    fn default() -> Self {
        Self::new(Value::default())
    }
}

impl<Value> CommonToken for ValueToken<Value> {
    type Value = Value;

    fn value(&self) -> &Value {
        &self.value
    }

    fn reset(&self) {}
}

pub struct CancelableToken<Token: CommonToken> {
    token: Token,
    canceled: AtomicFlag,
}

impl<Token: CommonToken> CancelableToken<Token> {
    pub fn new(token: Token) -> Self {
        CancelableToken {
            token,
            canceled: AtomicFlag::unset(),
        }
    }
}

impl<Token: CommonToken + Default> Default for CancelableToken<Token> {
    fn default() -> Self {
        Self::new(Token::default())
    }
}

impl<Token: CommonToken> CommonToken for CancelableToken<Token> {
    type Value = Token::Value;

    fn value(&self) -> &Self::Value {
        self.token.value()
    }

    fn reset(&self) {
        self.canceled.reset();
        self.token.reset();
    }
}

impl<T: CommonToken> Cancelable for CancelableToken<T> {
    fn cancel(&self) {
        self.canceled.set();
    }

    fn is_canceled(&self) -> bool {
        self.canceled.is_set()
    }
}

impl<T: CommonToken + Completable> Completable for CancelableToken<T> {
    fn complete(&self) {
        self.token.complete();
    }

    fn is_completed(&self) -> bool {
        self.token.is_completed()
    }
}

pub struct CompletableToken<Token: CommonToken> {
    token: Token,
    completed: AtomicFlag,
}

impl<Token: CommonToken> CompletableToken<Token> {
    pub fn new(token: Token) -> Self {
        CompletableToken {
            token,
            completed: AtomicFlag::unset(),
        }
    }
}

impl<Token: CommonToken + Default> Default for CompletableToken<Token> {
    fn default() -> Self {
        Self::new(Token::default())
    }
}

impl<Token: CommonToken> CommonToken for CompletableToken<Token> {
    type Value = Token::Value;

    fn value(&self) -> &Self::Value {
        self.token.value()
    }

    fn reset(&self) {
        self.completed.reset();
        self.token.reset();
    }
}

impl<T: CommonToken + Cancelable> Cancelable for CompletableToken<T> {
    fn cancel(&self) {
        self.token.cancel();
    }

    fn is_canceled(&self) -> bool {
        self.token.is_canceled()
    }
}

impl<T: CommonToken> Completable for CompletableToken<T> {
    fn complete(&self) {
        self.completed.set();
    }

    fn is_completed(&self) -> bool {
        self.completed.is_set()
    }
}

pub struct Token<S>(Arc<S>);

impl<S> Token<S> {
    pub fn new(state: S) -> Self {
        Token(Arc::new(state))
    }
}

impl<S> Clone for Token<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S> Deref for Token<S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<S: Default> Default for Token<S> {
    fn default() -> Self {
        Self(Arc::new(S::default()))
    }
}

pub struct TokenCompleter<T: Completable> {
    token: Token<T>,
}

impl<T: Completable> TokenCompleter<T> {
    pub fn new(token: Token<T>) -> Self {
        TokenCompleter { token }
    }

    pub fn token(&self) -> &T {
        &self.token
    }
}

impl<T: Completable> Drop for TokenCompleter<T> {
    fn drop(&mut self) {
        self.token.complete()
    }
}
