import XCTest
@testable import PortholeHelper

final class LoginItemRegistrarTests: XCTestCase {
    private final class CallCounter: @unchecked Sendable {
        private let lock = NSLock()
        private var count = 0

        func increment() {
            lock.lock()
            defer { lock.unlock() }
            count += 1
        }

        func snapshot() -> Int {
            lock.lock()
            defer { lock.unlock() }
            return count
        }
    }

    func testEnabledServiceDoesNotRegisterAgain() {
        let calls = CallCounter()
        let registrar = LoginItemRegistrar(
            dependencies: .init(
                status: { .enabled },
                register: { calls.increment() }
            )
        )

        XCTAssertEqual(registrar.registerIfNeeded(), .alreadyEnabled)
        XCTAssertEqual(calls.snapshot(), 0)
    }

    func testRequiresApprovalDoesNotRegisterAgain() {
        let calls = CallCounter()
        let registrar = LoginItemRegistrar(
            dependencies: .init(
                status: { .requiresApproval },
                register: { calls.increment() }
            )
        )

        XCTAssertEqual(registrar.registerIfNeeded(), .requiresApproval)
        XCTAssertEqual(calls.snapshot(), 0)
    }

    func testNotRegisteredServiceRegisters() {
        var statuses: [LoginItemRegistrar.ServiceStatus] = [.notRegistered, .enabled]
        let calls = CallCounter()
        let registrar = LoginItemRegistrar(
            dependencies: .init(
                status: { statuses.removeFirst() },
                register: { calls.increment() }
            )
        )

        XCTAssertEqual(registrar.registerIfNeeded(), .registered)
        XCTAssertEqual(calls.snapshot(), 1)
    }

    func testNotFoundServiceRegisters() {
        var statuses: [LoginItemRegistrar.ServiceStatus] = [.notFound, .enabled]
        let calls = CallCounter()
        let registrar = LoginItemRegistrar(
            dependencies: .init(
                status: { statuses.removeFirst() },
                register: { calls.increment() }
            )
        )

        XCTAssertEqual(registrar.registerIfNeeded(), .registered)
        XCTAssertEqual(calls.snapshot(), 1)
    }

    func testUnknownServiceStatusDoesNotRegister() {
        let calls = CallCounter()
        let registrar = LoginItemRegistrar(
            dependencies: .init(
                status: { .unknown },
                register: { calls.increment() }
            )
        )

        XCTAssertEqual(registrar.registerIfNeeded(), .failed("unknown SMAppService status"))
        XCTAssertEqual(calls.snapshot(), 0)
    }

    func testRegistrationFailureReportsFailure() {
        struct RegistrationError: Error, CustomStringConvertible {
            var description: String { "invalid signature" }
        }

        let registrar = LoginItemRegistrar(
            dependencies: .init(
                status: { .notRegistered },
                register: { throw RegistrationError() }
            )
        )

        XCTAssertEqual(registrar.registerIfNeeded(), .failed("invalid signature"))
    }

    func testRegistrationFailureReportsRequiresApprovalWhenStatusChanges() {
        var statuses: [LoginItemRegistrar.ServiceStatus] = [.notRegistered, .requiresApproval]
        let registrar = LoginItemRegistrar(
            dependencies: .init(
                status: { statuses.removeFirst() },
                register: { throw NSError(domain: "SM", code: 10) }
            )
        )

        XCTAssertEqual(registrar.registerIfNeeded(), .requiresApproval)
    }
}
