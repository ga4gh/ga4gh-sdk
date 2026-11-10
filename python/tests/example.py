import traceback
from GA4GH import Configuration, ServiceType, TES

try:
    config = Configuration("https://localhost") # legacy parameter, will be removed
    config.from_file(ServiceType.TES) # load configuration from ~/.ga4gh-cli/config.json

    TES = TES(config)
    print("Creating task...", end="")
    task = TES.create('../tests/sample.tes')
    if not task:
        print("Task could not be created\n")
        exit(1)

    print("Task created successfully:")
    print("TASK ID: ", task.id)
    print("STATUS:  ", task.status())
    print("Canceling task...")
    if not task.cancel():
        print("Task could not be canceled\n")
        exit(1)
    print("Task canceled successfully\n")

    print("Listing tasks...")
    print(f"{'TASK ID':<25} {'STATUS':<15}")
    for task in TES.list().tasks:
        print(f"{task.id:<25} {task.status:<15}")

except Exception as e:
    print("An error occurred:")
    traceback.print_exc()
