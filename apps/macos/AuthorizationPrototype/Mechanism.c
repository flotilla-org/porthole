// Throwaway custom-right probe. No taskport, login, TCC or requester policy.
#include <Security/AuthorizationPlugin.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

#define BROKER_SOCKET "/var/run/porthole-auth-prototype/broker.sock"
typedef struct { const AuthorizationCallbacks *callbacks; } Plugin;
typedef struct {
    Plugin *plugin;
    AuthorizationEngineRef engine;
    pthread_mutex_t lock;
    pthread_t thread;
    bool started, invoked, cancelled;
    int socket;
} Mechanism;

static void *evaluate(void *argument) {
    Mechanism *m = argument;
    int fd = socket(AF_UNIX, SOCK_STREAM, 0);
    bool allow = false;
    if (fd >= 0) {
        int no_sigpipe = 1;
        struct timeval timeout = {.tv_sec = 65};
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &no_sigpipe, sizeof(no_sigpipe));
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout));
        setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout));
        pthread_mutex_lock(&m->lock);
        m->socket = fd;
        bool cancelled = m->cancelled;
        pthread_mutex_unlock(&m->lock);
        struct sockaddr_un address = {.sun_len = sizeof(address), .sun_family = AF_UNIX};
        strlcpy(address.sun_path, BROKER_SOCKET, sizeof(address.sun_path));
        uid_t uid; gid_t gid;
        if (!cancelled && connect(fd, (struct sockaddr *)&address, sizeof(address)) == 0 &&
            getpeereid(fd, &uid, &gid) == 0 && uid == 0) {
            const char request[] = "{\"command\":\"request\"}\n";
            if (send(fd, request, sizeof(request) - 1, 0) == sizeof(request) - 1) {
                char response[16] = {0};
                size_t length = 0;
                while (length < sizeof(response) - 1) {
                    ssize_t count = recv(fd, response + length, 1, 0);
                    if (count != 1 || response[length++] == '\n') break;
                }
                allow = strcmp(response, "ALLOW\n") == 0;
            }
        }
        pthread_mutex_lock(&m->lock);
        m->socket = -1;
        close(fd);
        pthread_mutex_unlock(&m->lock);
    }
    pthread_mutex_lock(&m->lock);
    bool deliver = !m->cancelled;
    pthread_mutex_unlock(&m->lock);
    // No framework callback under our mutex. Deactivate joins before notifying
    // the engine, so no worker result can follow DidDeactivate.
    if (deliver)
        m->plugin->callbacks->SetResult(m->engine, allow ? kAuthorizationResultAllow : kAuthorizationResultDeny);
    return NULL;
}
static OSStatus destroy_plugin(AuthorizationPluginRef p) { free(p); return errAuthorizationSuccess; }
static OSStatus create_mechanism(AuthorizationPluginRef p, AuthorizationEngineRef engine,
                                AuthorizationMechanismId name, AuthorizationMechanismRef *out) {
    if (strcmp(name, "approve") != 0) return errAuthorizationInternal;
    Mechanism *m = calloc(1, sizeof(*m));
    if (!m) return errAuthorizationInternal;
    m->plugin = p; m->engine = engine; m->socket = -1;
    pthread_mutex_init(&m->lock, NULL);
    *out = m;
    return errAuthorizationSuccess;
}
static OSStatus invoke(AuthorizationMechanismRef reference) {
    Mechanism *m = reference;
    // Publish worker state before the thread can deliver a result/reenter.
    pthread_mutex_lock(&m->lock);
    bool fail = m->invoked;
    m->invoked = true;
    if (!fail) {
        fail = pthread_create(&m->thread, NULL, evaluate, m) != 0;
        m->started = !fail;
    }
    pthread_mutex_unlock(&m->lock);
    if (fail) return m->plugin->callbacks->SetResult(m->engine, kAuthorizationResultDeny);
    return errAuthorizationSuccess;
}

static void cancel(Mechanism *m) {
    pthread_mutex_lock(&m->lock);
    m->cancelled = true;
    if (m->socket >= 0) shutdown(m->socket, SHUT_RDWR);
    pthread_mutex_unlock(&m->lock);
}
static void finish_worker(Mechanism *m) {
    if (m->started) {
        // A synchronous callback can reenter on this worker; it has no further
        // mechanism accesses after SetResult returns.
        if (!pthread_equal(m->thread, pthread_self())) pthread_join(m->thread, NULL);
        else pthread_detach(m->thread);
        m->started = false;
    }
}
static OSStatus deactivate(AuthorizationMechanismRef reference) {
    Mechanism *m = reference;
    cancel(m);
    finish_worker(m);
    return m->plugin->callbacks->DidDeactivate(m->engine);
}
static OSStatus destroy_mechanism(AuthorizationMechanismRef reference) {
    Mechanism *m = reference;
    cancel(m);
    finish_worker(m);
    pthread_mutex_destroy(&m->lock);
    free(m);
    return errAuthorizationSuccess;
}
static const AuthorizationPluginInterface interface = {
    kAuthorizationPluginInterfaceVersion, destroy_plugin, create_mechanism,
    invoke, deactivate, destroy_mechanism
};
OSStatus AuthorizationPluginCreate(const AuthorizationCallbacks *callbacks,
                                  AuthorizationPluginRef *plugin,
                                  const AuthorizationPluginInterface **out) {
    Plugin *p = calloc(1, sizeof(*p));
    if (!p) return errAuthorizationInternal;
    p->callbacks = callbacks;
    *plugin = p; *out = &interface;
    return errAuthorizationSuccess;
}
